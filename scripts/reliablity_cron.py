#! python3

"""
This program reaps expired records and adjusts counts.

Specifying "--nap=0" will cause this app to only run once.

"""

import argparse
import asyncio
import json
import logging
import os
import pdb
import time
from typing import cast

import redis
import statsd
import toml
from google.cloud.bigtable.data import (
    BigtableDataClientAsync,
    RowMutationEntry,
    SetCell,
)


class Counter:
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
            import pdb

            pdb.set_trace()
            self.redis = redis.Redis.from_url(settings.reliability_dsn)
            self.bigtable = BigtableDataClientAsync(
                project=settings.bigtable["project"]
            )
            self.log = log
            self.settings = settings
        except Exception as e:
            log.error(e)

    async def gc(self) -> dict[str, int | float]:
        """Prune expired elements, decrementing counters and logging result"""
        start = time.time()
        # The table of counts
        counts = self.settings.count_table
        # The table of expirations
        expiry = self.settings.expiry_table
        # the BigTable reliability family
        log_family = self.settings.log_family

        # Fetch the candidates to purge.
        pdb.set_trace()
        mutations = list()
        purged = cast(
            list[bytes], self.redis.zrange(expiry, -1, int(start), byscore=True)
        )
        # Fix up the counts
        with self.redis.pipeline() as pipeline:
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
                pipeline.execute()
                # then add the bigtable logs
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

    def counts(self) -> dict[str, int]:
        """Return the current milestone counts (this should happen shortly after a gc)"""
        return cast(dict[str, int], self.redis.hgetall(self.settings.count_table))


def record_metrics(
    log: logging.Logger, settings: argparse.Namespace, counts: dict[str, int]
):
    """Record the counts to metrics"""
    log.info(f"📈 Recording metrics: {counts}")
    for label, count in counts.items():
        cast(statsd.StatsClient, settings.metric).gauge(label, count)


def config(env_args: os._Environ = os.environ) -> argparse.Namespace:
    """Read the configuration from the args and environment."""
    parser = argparse.ArgumentParser(
        description="Manage Autopush Reliability Tracking Redis data."
    )
    parser.add_argument("-c", "--config", help="configuration_file", action="append")
    parser.add_argument(
        "--reliability_dsn",
        "-r",
        help="DSN to connect to the Redis like service.",
        default=env_args.get(
            "AUTOEND_RELIABILITY_DSN", env_args.get("AUTOCONNECT_RELIABILITY_DSN")
        ),
    )
    parser.add_argument(
        "--db_dsn",
        "-b",
        help="User Agent ID",
        default=env_args.get("AUTOEND_DB_DSN", env_args.get("AUTOCONNECT_DB_DSN")),
    )
    parser.add_argument(
        "--db_settings",
        "-s",
        help="User Agent ID",
        default=env_args.get(
            "AUTOEND_DB_SETTINGS", env_args.get("AUTOCONNECT_DB_SETTINGS")
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
        "--log_family",
        help="Name of Bigtable log family",
        default=env_args.get("AUTOTRACK_EXPIRY", "reliability"),
    )
    parser.add_argument(
        "--statsd_host",
        help="Metric host name",
        default=env_args.get(
            "AUTOEND_STATSD_HOST", env_args.get("AUTOCONNECT_STATSD_HOST")
        ),
    )
    parser.add_argument(
        "--statsd_port",
        help="Metric host port",
        default=env_args.get(
            "AUTOEND_STATSD_HOST", env_args.get("AUTOCONNECT_STATSD_HOST", 8125)
        ),
    )
    parser.add_argument(
        "--statsd_label",
        help="Metric root namespace",
        default=env_args.get(
            "AUTOEND_STATSD_LABEL",
            env_args.get("AUTOCONNECT_STATSD_LABEL", "autotrack"),
        ),
    )
    parser.add_argument(
        "--nap",
        help="seconds to nap between each gc cycle (smaller number is more accurate measurements)",
        default=60,
    )
    args = parser.parse_args()

    # if we have a config file, read from that and then reload.
    if args.config is not None:
        for filename in args.config:
            with open(filename, "r") as f:
                args = parser.set_defaults(**toml.load(f))
    args = parser.parse_args()

    # fixup the bigtable settings so that they're easier for this script to deal with.
    if args.db_settings is not None:
        bt_settings = json.loads(args.db_settings)
        parts = bt_settings.get("table_name").split("/")
        for i in range(0, len(parts), 2):
            # remember: the `tablename` dsn uses plurals for
            # `projects`, `instances`, & `tables`
            bt_settings[parts[i].rstrip("s")] = parts[i + 1]
        args.bigtable = bt_settings

    if args.statsd_host or args.statsd_port:
        args.metrics = statsd.StatsClient(
            args.statsd_host, args.statsd_port, prefix=args.statsd_label
        )
    else:
        args.metrics = None

    return args


def init_logs():
    """Initialize logging (based on `PYTHON_LOG` environ)"""
    level = getattr(logging, os.environ.get("PYTHON_LOG", "INFO").upper(), None)
    logging.basicConfig(level=level)
    log = logging.getLogger("autotrack")
    return log


async def amain(log: logging.Logger, settings: argparse.Namespace):
    """Async main loop"""
    counter = Counter(log, settings)
    while True:
        _result = await counter.gc()
        record_metrics(log, settings, counter.counts())
        # TODO: adjust timing loop based on result time.
        # Ideally, this would have a loop that it runs on that
        # becomes tighter the more items were purged, and adjusts
        # based on the time it took to run.
        if settings.nap == 0:
            return
        time.sleep(settings.nap)


def main():
    """Configure and start async main loop"""
    log = init_logs()
    log.info("Starting up...")
    asyncio.run(amain(log, config()))


if __name__ == "__main__":
    main()
