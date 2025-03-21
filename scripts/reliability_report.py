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
import time
from typing import cast
from collections import OrderedDict, defaultdict
from datetime import datetime

import jinja2
import toml
from google.cloud.bigtable.data import (
    BigtableDataClientAsync,
    ReadRowsQuery,
    RowRange,
)
from google.cloud.bigtable.data.row_filters import FamilyNameRegexFilter

RELIABILITY_FAMILY = "reliability"

"""
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

    async def collect(self):
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
            for index, timestamp in enumerate(milestone_keys):
                milestone = milestones.get(timestamp)
                try:
                    operation_length = list(milestone_keys)[index + 1] - timestamp
                except IndexError:
                    operation_length = 0
                log = milestone_log[milestone]
                log["count"] += 1
                log["total_time"] += operation_length
                log["average_time"] = (log["average_time"] + operation_length) / log[
                    "count"
                ]

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
            since=datetime.fromtimestamp(start_time),
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

    async def output(self, output_type):
        match output_type.lower():
            case "json":
                return await self.as_json()
            case "raw":
                return await self.as_raw()
            case _:
                return await self.as_md()

    async def as_raw(self):
        """Return the data as a raw python dict dump"""
        print(await self.collect())

    async def as_json(self):
        """Return as a formatted JSON string"""
        collected = await self.collect()
        meta = collected.get("meta")
        try:
            # convert the instance to a string.
            cast(OrderedDict, meta)["since"] = cast(
                datetime, cast(OrderedDict, meta).get("since", datetime.now())
            ).strftime("%Y-%m-%d %H:%M:%S.%f %Z")
        except Exception as ex:
            self.log.error(ex)
        return json.dumps(meta, indent=2)

    async def as_md(self):
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


def config(env_args: os._Environ = os.environ) -> argparse.Namespace:
    """Read the configuration from the args and environment."""
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
        "--output",
        help="Output format: md (MarkDown text), json (Formatted JSON), raw (python dump)",
        default="md",
        choices=["md", "json", "raw"],
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
    print(await bigtable.output(settings.output))


def main():
    """Configure and start async main loop"""
    log = init_logs()
    log.info("Starting up...")
    asyncio.run(amain(log, config()))


if __name__ == "__main__":
    main()
