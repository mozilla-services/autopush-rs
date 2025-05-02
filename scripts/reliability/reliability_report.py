#! /bin/env python3

"""
Generate longer form reports from the collected reliability logging data.

Because this collects all rows in the `reliability` family, it is NOT
recommended for frequent execution. (Daily should be sufficient.)

"""

import argparse
import asyncio
import datetime
import json
import logging
import os
import statistics
import time
from typing import Any, cast, Dict, List
from collections import OrderedDict, defaultdict
from datetime import datetime, timedelta, timezone

import jinja2
import redis.asyncio as redis_async
import toml
from google.cloud.bigtable.data import (
    BigtableDataClientAsync,
    RowMutationEntry,
    ReadRowsQuery,
    RowRange,
    SetCell,
)
from google.cloud.bigtable.data.row_filters import FamilyNameRegexFilter
from google.cloud import storage

RELIABILITY_FAMILY = "reliability"

"""
This file is a combination tool that performs daily maintenance of the Push Reliability
data, as well as generate a daily report of the stats. These reports are stored in a
GCP Storage Bucket in both JSON and human readable Markdown format, by default. Originally,
these were separate scripts, but due to the daily requirement for both, it was decided to
merge scripts into one tool.

Reliability data is stored in bigtable under the `reliability_id` key.
The `reliability_id` is an opaque string that can be passed from an outside source,
but by default it's a UUID4() string, so we can make some presumptions.
For now, we can presume that the string will follow most ASCII formats, so all characters
will be between `/` and `{`. Added bonus is that bigtable will allow us to do "partial"
lookups, so we can fetch all by looking at the range.
"""


class BigtableScanner:
    """Bigtable access class"""

    def __init__(self, log: logging.Logger, settings: argparse.Namespace):
        try:
            self.bigtable = BigtableDataClientAsync(
                project=settings.bigtable["project"]
            )
            self.log = log
            self.settings = settings
        except Exception as e:
            log.error(e)
            raise

    async def collect(self) -> Dict[str, Any]:
        """Collect the data elements. (NOTE: This does a table scan. )"""
        table = self.bigtable.get_table(
            self.settings.bigtable.get("instance"),
            self.settings.bigtable.get("table"),
        )

        # The key is a semi-opaque string value that may be closely related to a
        # UUID4. This was chosen because time based keys can cause issues with
        # bigtable.
        # We can potentially break this into sub queries, but for now, treat
        # it as a partial string that encompasses all values.
        rrange = RowRange(start_key="/", end_key="{")
        rfilter = FamilyNameRegexFilter(self.settings.track_family)
        rrquery = ReadRowsQuery(row_ranges=rrange, row_filter=rfilter)

        raw_rows = await table.read_rows(query=rrquery)
        result = {}
        # Meta information about each record
        max_time = mean_time = 0
        min_time = 1000
        start_time = time.time()
        milestone_log = defaultdict(lambda: dict(count=0, total_time=0, average_time=0))

        # Meta information about all records
        success_count = fail_count = row_count = expired_count = 0

        # Iterate over the rows and collect interesting bits of info
        for row_count, row in enumerate(raw_rows, start=1):
            milestones = OrderedDict()
            successful = expired = False
            total_time = 0
            for cell in row.cells:
                milestone = bytearray(cell.qualifier).decode()
                timestamp = int.from_bytes(cell.value) * 0.001
                if timestamp < start_time:
                    start_time = timestamp
                # record by timestamp so we can determine how long things took
                # as well as what order milestones were reached.
                milestones[timestamp] = milestone
                # if we've made it to either "transmitted" or "delivered"
                # consider it a success.
                if milestone in ("delivered", "transmitted"):
                    successful |= True
                else:
                    # Records are marked as "expired" by the `gc` function.
                    expired |= milestone == "expired"
            timed_milestones = OrderedDict(sorted(milestones.items()))
            milestone_keys = timed_milestones.keys()
            times = []
            for index, timestamp in enumerate(milestone_keys):
                milestone = milestones.get(timestamp)
                try:
                    operation_length = list(milestone_keys)[index + 1] - timestamp
                except IndexError:
                    operation_length = 0
                log = milestone_log[milestone]
                log["count"] += 1
                log["total_time"] += operation_length
                times.append(operation_length)
            log["average_time"] = statistics.mean(times)

            key_list = list(timed_milestones.keys())
            total_time = key_list[-1] - key_list[0]
            if successful:
                if total_time < min_time:
                    min_time = total_time
                if total_time > max_time:
                    max_time = total_time
                success_count += 1
            else:
                fail_count += 1
            if expired:
                expired_count += 1
            mean_time = mean_time + total_time / row_count
            # Should we ever want to return "all":
            # result[row.row_key] = print_row
        result["meta"] = OrderedDict(
            since=datetime.fromtimestamp(start_time).isoformat(),
            longest=max_time,
            shortest=min_time,
            mean_time=mean_time,
            success_count=success_count,
            fail_count=fail_count,
            total_count=row_count,
            expired_count=expired_count,
            milestones=milestone_log,
        )
        return result

    async def output(self, output_type) -> str:
        match output_type.lower():
            case "json":
                return await self.as_json()
            case "raw":
                return await self.as_raw()
            case _:
                return await self.as_md()

    async def as_raw(self) -> str:
        """Return the data as a raw python dict dump"""
        return str(await self.collect())

    async def as_json(self) -> str:
        """Return as a formatted JSON string"""
        collected = await self.collect()
        meta = cast(OrderedDict, collected.get("meta"))
        # ensure the "since" is set.
        meta["since"] = meta.get("since", datetime.now().isoformat)
        return json.dumps(meta, indent=2)

    async def as_md(self) -> str:
        """Return as a MarkDown formatted document"""
        env = jinja2.Environment(
            loader=jinja2.PackageLoader("reliability_report"),
            autoescape=jinja2.select_autoescape(),
        )
        template = env.get_template("reliable_report_template.md")
        data = await self.collect()
        return template.render(
            date=datetime.now().strftime(format="%Y-%m-%d %H:%M:%S %Z"),
            meta=data.get("meta"),
        )


class Redis:
    """Manage Redis-like storage counts

    Current milestone counts are managed in a Redis-like storage system.
    There are two parts required, one is the active milestone count (as
    an HINCR). The other is a ZHash that contains the expiration
    timestamp for records.
    Our 'garbage collection' goes through the ZHash looking for expired
    records and removes them while decrementing the associated HINCR count,
    indicating that the record expired "in place".

    We also update the Bigtable message log indicating that a message
    failed to be delivered.
    """

    def __init__(self, log: logging.Logger, settings: argparse.Namespace):
        try:
            self.redis: redis_async.Redis = redis_async.Redis.from_url(
                settings.reliability_dsn
            )
            self.bigtable = BigtableDataClientAsync(
                project=settings.bigtable["project"]
            )
            self.log = log
            self.settings = settings
        except Exception as e:
            log.error(e)

    async def gc(self) -> Dict[str, int | float]:
        """Perform quick garbage collection. This includes pruning expired elements,
        decrementing counters and potentially logging the result.
        This sort of garbage collection should happen quite frequently.
        """
        start = time.time()
        # The table of counts
        counts = self.settings.count_table
        # The table of expirations
        expiry = self.settings.expiry_table
        # the BigTable reliability family
        log_family = self.settings.log_family

        # Fetch the candidates to purge.
        mutations = list()
        purged = cast(
            list[bytes], await self.redis.zrange(expiry, -1, int(start), byscore=True)
        )
        # Fix up the counts
        async with self.redis.pipeline() as pipeline:
            for key in purged:
                # clean up the counts.
                parts = key.split(b"#", 2)
                state = parts[0]
                self.log.debug(f"🪦 decr {state.decode()}")
                pipeline.hincrby(counts, state.decode(), -1)
                pipeline.zrem(expiry, key)
                # and add the log info.
                mutations.append(
                    RowMutationEntry(
                        key, SetCell(log_family, "expired", int(start * 1000))
                    )
                )
                mutations.append(
                    RowMutationEntry(
                        key,
                        SetCell(
                            log_family,
                            "error",
                            "expired",
                        ),
                    )
                )
            if len(purged) > 0:
                # make the changes to redis,
                await pipeline.execute()
                # then add the bigtable logs
                if self.bigtable:
                    table = self.bigtable.get_table(
                        self.settings.bigtable.get("instance"),
                        self.settings.bigtable.get("table"),
                    )
                    await table.bulk_mutate_rows(mutations)

        result = {
            "trimmed": len(purged),
            "time": int(start * 1000) - (time.time() * 1000),
        }
        if len(purged):
            self.log.info(
                f"🪦 Trimmed {result.get("trimmed")} in {result.get("time")}ms"
            )
        return result

    async def get_counts(self) -> Dict[str, int]:
        """Return the current milestone counts (this should happen shortly after a gc)"""
        return cast(dict[str, int], await self.redis.hgetall(self.settings.count_table))

    async def adjust_counts(
        self,
        start_of_day: datetime,
    ):
        """Adjust counts to remove records we no longer care about."""
        try:
            self.log.info(f"🧹 Generating daily snapshot for {start_of_day}")
            # get all the unresolved terminals (since we remove them after we process them.)
            terminals = await self.redis.zrange(
                self.settings.terminal_table,
                -1,
                int(start_of_day.timestamp()),
                withscores=True,
            )
            if not terminals:
                self.log.warning("🧹 No old data found, nothing to clean")
                return
            self.log.debug(f"🧹 Clearing Terminals: {terminals}")
            pipe = self.redis.pipeline()
            for terms, score in terminals:
                try:
                    vals = json.loads(terms)
                    self.log.debug(f"🧹 For {score}, clearing: {vals}")
                    for key in vals:
                        pipe.hincrby(self.settings.count_table, key, 0 - vals.get(key))
                except ValueError as e:
                    self.log.error(f"Could not parse terminal line: {e} :: {terms}")
                except Exception as e:
                    self.log.error(f"Unknown error occurred in decrement: {e}")
                    raise
            if terminals:
                start_score = terminals[0][1]
                end_score = terminals[-1][1]
                self.log.debug(
                    f"🧹 Clearing from {start_score} to {end_score} from {self.settings.terminal_table}"
                )
                pipe.zremrangebyscore(
                    self.settings.terminal_table, int(start_score), int(end_score)
                )
            # automatically executes as a transaction.
            result = await pipe.execute()
            self.log.debug(f"🧹 Result: {result}")
        except Exception as e:
            self.log.error(f"Unknown error in terminal_snapshot {e}")
            raise

    async def terminal_snapshot(self) -> Dict[str, int]:
        """Take a daily snapshot of the terminal counts for message states.
        "Terminal" means that the message has reached it's presumed final state, and that
        no further state change is possible. These are cleared after the max holding period to
        prevent the terminal counts from endlessly incrementing.
        """
        start_of_day = datetime.now(tz=timezone.utc).replace(
            hour=0, minute=0, second=0, microsecond=0
        )

        # These are defined in autopush-common::reliability::ReliabilityState::is_terminal().
        # Make sure to use the `snake_case` string variant.
        terminal_states = [
            "decryption_error",
            "delivered",
            "errored",
            "expired",
            "not_delivered",
        ]

        # Check to see if there are any pending fixups we need to process.
        await self.adjust_counts(start_of_day)

        # Bulk get the current counts and then store them into the terminal table.
        # Since we're getting all the counts in one action, no transaction is required.
        vals = await self.redis.hmget(self.settings.count_table, terminal_states)
        # redis returns a str:bytes, so can't just call dict(vals)
        term_counts = {
            k: v for (k, v) in dict(zip(terminal_states, vals)).items() if v is not None
        }
        if term_counts:
            self.log.info(
                f"🧹 Recording expired data: {term_counts} for {start_of_day}"
            )
            expiry = start_of_day + timedelta(days=self.settings.max_retention)
            # Write the values to storage.
            await self.redis.zadd(
                self.settings.terminal_table,
                {str(int(expiry.timestamp())): json.dumps(term_counts)},
            )
        else:
            self.log.info(f"🧹 No matching terminal states found?")
        return term_counts

    async def get_lock(self) -> bool:
        """Use RedLock locking"""
        lock_name = datetime.now().isoformat()
        # set the default hold time fairly short, we'll extend the lock later if we succeed.
        self.lock = self.redis.lock(lock_name, timeout=self.settings.lock_acquire_time)
        # Fail the lock check quickly.
        if await self.lock.acquire(blocking_timeout=1):
            await self.lock.extend(self.settings.lock_hold_time)
            return True
        return False

    async def release_lock(self):
        await self.lock.release()


async def write_report(
    log: logging.Logger,
    settings: argparse.Namespace,
    bigtable: BigtableScanner,
    bucket: storage.Bucket,
    report_name: str,
):
    """Write the reports to the bucket"""
    for style in settings.output:
        blob_name = f"{report_name}.{style}"
        log.info(f"Creating {blob_name}")
        bucket.blob(blob_name).open("w").write(await bigtable.output(style))


async def clean_bucket(
    log: logging.Logger, settings: argparse.Namespace, bucket: storage.Bucket
):
    """Remove old reports from the bucket"""
    end_offset = datetime.now().strftime("%Y-%m-%d")
    start_date = (
        datetime.now() - timedelta(days=settings.bucket_retention_days)
    ).strftime("%Y-%m-%d")
    blobs = bucket.list_blobs(start_offset=start_date, end_offset=end_offset)
    for blob in blobs:
        log.info(f"🗑 Deleting {blob.name} -> {blob.id}")
        blob.delete()


def config(env_args: os._Environ = os.environ) -> argparse.Namespace:
    """Read the configuration from the args and environment."""
    report_formats = ["json", "md", "raw"]

    parser = argparse.ArgumentParser(
        description="Manage Autopush Reliability Tracking Redis data."
    )

    parser.add_argument("-c", "--config", help="configuration_file", action="append")

    parser.add_argument(
        "--db_dsn",
        "-b",
        help="Database DSN connection string",
        default=env_args.get(
            "AUTOEND__DB_DSN",
            env_args.get("AUTOCONNECT__DB_DSN", "grpc://localhost:8086"),
        ),
    )

    parser.add_argument(
        "--db_settings",
        "-s",
        help="Database settings",
        default=env_args.get(
            "AUTOEND__DB_SETTINGS",
            env_args.get(
                "AUTOCONNECT__DB_SETTINGS",
                '{"message_family":"message","router_family":"router", "table_name":"projects/test/instances/test/tables/autopush"}',
            ),
        ),
    )

    parser.add_argument(
        "--track_family",
        help="Name of Bigtable reliability logging family",
        default=env_args.get("AUTOTRACK_FAMILY", RELIABILITY_FAMILY),
    )

    parser.add_argument(
        "--bucket_name",
        help="GCP Storage Bucket name",
        default=env_args.get("AUTOTRACK_BUCKET_NAME"),
    )

    parser.add_argument(
        "--bucket_retention_days",
        help="Number of days to retain generated reports in the Bucket",
        default=env_args.get("AUTOTRACK_BUCKET_RETENTION_DAYS", 30),
    )

    parser.add_argument(
        "--output",
        help="Output formats: md (MarkDown text), json (Formatted JSON), raw (python dump) (e.g. `--output md json`)",
        nargs="+",
        action="extend",
        # SEE DEFAULT HANDLER BELOW
        choices=report_formats,
    )

    parser.add_argument(
        "--reliability_dsn",
        "-r",
        help="DSN to connect to the Redis like service.",
        default=env_args.get(
            "AUTOEND__RELIABILITY_DSN",
            env_args.get("AUTOCONNECT__RELIABILITY_DSN", "redis://localhost"),
        ),
    )

    parser.add_argument(
        "--count_table",
        help="Name of Redis table of milestone counts",
        default=env_args.get("AUTOTRACK_COUNTS", "state_counts"),
    )

    parser.add_argument(
        "--expiry_table",
        help="Name of Redis table of milestone expirations",
        default=env_args.get("AUTOTRACK_EXPIRY", "expiry"),
    )

    parser.add_argument(
        "--terminal_table",
        help="Name of the terminal snap-shot table",
        default=env_args.get("AUTOTRACK_TERMINAL_TABLE", "terminus"),
    )

    parser.add_argument(
        "--log_family",
        help="Name of the reliability report logging family, must match `autopush_common::db::bigtable::bigtable_client::RELIABLE_LOG_FAMILY`",
        default=env_args.get("AUTOTRACK_LOG_FAMILY", "reliability"),
    )

    # A note about the retention period.
    # Redis is not reliable storage. The system can go down for any number of reasons.
    # In order to not tempt the fates more than necessary, we'll "trim" the terminated
    # records after 24 hours. If you want to look at any longer range data, that's what
    # the daily report is for.
    parser.add_argument(
        "--terminal_max_retention_days",
        help="Number of days that data will be retained",
        default=env_args.get("AUTOTRACK_TERM_MAX_RETENTION_DAYS", 1),
    )

    parser.add_argument(
        "--report_max_retention_days",
        help="Number of days to retain reports in the reliability bucket",
        default=env_args.get("AUTOTRACK_REPORT_MAX_RETENTION_DAYS", 30),
    )

    parser.add_argument(
        "--report_bucket_name",
        help="Name of the bucket to store reliability reports",
        default=env_args.get(
            "AUTOTRACK_REPORT_BUCKET_NAME", "autopush-dev-reliability"
        ),
    )

    parser.add_argument(
        "--lock_hold_time",
        help="seconds to hold the lock, once acquired (lock expires in # seconds)",
        default=env_args.get("AUTOTRACK__LOCK_HOLD_TIME", 600),
    )

    parser.add_argument(
        "--lock_acquire_time",
        help="seconds to hold the lock for initial acquisition",
        default=env_args.get("AUTOTRACK__LOCK_ACQUISITION_TIME", 10),
    )

    args = parser.parse_args()

    # if we have a config file, read from that and then reload.
    if args.config is not None:
        for filename in args.config:
            with open(filename, "r") as f:
                args = parser.set_defaults(**toml.load(f))
    args = parser.parse_args()
    # fixup the bigtable settings so that they're easier for this script to deal with and
    # more human readable.
    if args.db_settings is not None:
        bt_settings = json.loads(args.db_settings)
        parts = bt_settings.get("table_name").split("/")
        for i in range(0, len(parts), 2):
            # remember: the `tablename` dsn uses plurals for
            # `projects`, `instances`, & `tables`
            bt_settings[parts[i].rstrip("s")] = parts[i + 1]
        args.bigtable = bt_settings
    # If we have a bucket, we'll write the report there.
    # Filter the provided output formats to ones we know.
    # The reports will be date stamped and stored in the bucket with the output prefixes.
    if args.report_bucket_name is not None:
        formats = os.environ.get("AUTOTRACK_OUTPUT", "md json")
        if formats:
            setattr(
                args,
                "output",
                list(filter(lambda x: x in report_formats, formats.split(" "))),
            )
    return args


def init_logs():
    """Initialize logging (based on `PYTHON_LOG` environ)"""
    level = getattr(logging, os.environ.get("PYTHON_LOG", "WARN").upper(), None)
    logging.basicConfig(level=level)
    log = logging.getLogger("autotrack")
    return log


async def amain(log: logging.Logger, settings: argparse.Namespace):
    """Async main loop"""
    bigtable = BigtableScanner(log, settings)
    # do the Redis counter cleanup.
    counter = Redis(log, settings)
    await counter.gc()
    if await counter.get_lock():
        await counter.terminal_snapshot()
        # if we have a bucket to write to, write the reports to the bucket.
        if settings.bucket_name:
            client = storage.Client()
            bucket = client.lookup_bucket(bucket_name=settings.bucket_name)
            if bucket:
                report_name = datetime.now().strftime("%Y-%m-%d")
                # technically, `bucket.list_blobs` can return `.num_results` but since this is
                # an iterator for the actual call and the call is only performed on demand, that
                # would return 0.
                reports = [blob for blob in bucket.list_blobs(prefix=report_name)]
                if len(reports) == 0:
                    if await counter.get_lock():
                        await write_report(log, settings, bigtable, bucket, report_name)
                        await clean_bucket(log, settings, bucket)
                        await counter.release_lock()
                    else:
                        log.debug("Could not get lock, skipping...")
                else:
                    log.debug("Reports already generated, skipping...")
    # Maybe we're just interested in getting a report?
    if not settings.bucket_name:
        for style in settings.output:
            print(await bigtable.output(style))
            print("\f")


# This can also act as a Flask app entry point.
def main():
    """Configure and start async main loop"""
    log = init_logs()
    log.info("Starting up...")
    asyncio.run(amain(log, config()))
    return "Ok"


if __name__ == "__main__":
    main()
