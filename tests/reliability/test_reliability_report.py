"""Tests for the Push Reliability report/maintenance script.

The Redis-backed tests cover the daily maintenance path, which is the only
place this script mutates the live milestone counters. They run against a real
Redis rather than a fake, because the defects they guard against are exact
Redis semantics -- chiefly `ZRANGE ... BYSCORE` versus the default rank-index
form -- that a reimplementation may not reproduce faithfully.

The module under test is `scripts/reliability/reliability_report.py`, put on
`sys.path` by this directory's `conftest.py`.

Requires a Redis server. Set `RELIABILITY_TEST_REDIS_DSN` to override the
default of `redis://localhost:6379`. Run via `make reliability-test`.
"""

import argparse
import json
import os
import sys
from datetime import datetime, timedelta, timezone

import pytest
import reliability_report

# A fixed day to anchor the tests; `terminal_snapshot` keys its behavior off
# the current UTC day, so the tests freeze it rather than racing midnight.
DAY_ONE = datetime(2026, 9, 17, 3, 30, tzinfo=timezone.utc)
START_OF_DAY_ONE = DAY_ONE.replace(hour=0, minute=0, second=0, microsecond=0)

REDIS_DSN = os.environ.get("RELIABILITY_TEST_REDIS_DSN", "redis://localhost:6379")


def freeze(monkeypatch, when: datetime) -> None:
    """Pin `reliability_report.datetime.now()` to `when`.

    The module does `from datetime import datetime`, so the name is patched on
    the module rather than on the datetime package. Subclassing keeps
    `fromtimestamp`, `replace` and arithmetic intact.
    """

    class Frozen(datetime):
        @classmethod
        def now(cls, tz=None):
            return when

    monkeypatch.setattr(reliability_report, "datetime", Frozen)


def make_settings() -> argparse.Namespace:
    """Build the settings namespace the Redis class expects.

    Deliberately not routed through `config()`: these tests pin the values they
    care about, and `config()` parses `sys.argv`.
    """
    return argparse.Namespace(
        reliability_dsn=REDIS_DSN,
        # Only `gc()` issues Bigtable calls, and none of these tests reach a
        # mutation, so the table coordinates just need to be well formed.
        bigtable={
            "project": "test-project",
            "instance": "test-instance",
            "table": "autopush",
        },
        count_table="state_counts",
        expiry_table="expiry",
        terminal_table="terminus",
        snapshot_marker="terminus_last_run",
        log_family="reliability",
        terminal_max_retention_days=1,
        lock_hold_time=600,
        lock_acquire_time=10,
    )


@pytest.fixture
async def client(monkeypatch):
    """Provide a `Redis` helper wired to a flushed test keyspace."""
    # The Bigtable client is constructed eagerly in `Redis.__init__` and would
    # otherwise hunt for application default credentials. Pointing it at an
    # emulator keeps construction credential-free; nothing here connects.
    monkeypatch.setenv("BIGTABLE_EMULATOR_HOST", "localhost:8086")

    log = reliability_report.logging.getLogger("autotrack-test")
    instance = reliability_report.Redis(log, make_settings())
    if not hasattr(instance, "redis"):
        pytest.skip(f"could not construct a Redis client for {REDIS_DSN}")
    try:
        await instance.redis.ping()
    except Exception as exc:  # pragma: no cover - environment guard
        pytest.skip(f"no Redis at {REDIS_DSN}: {exc}")
    await instance.redis.flushdb()
    yield instance
    await instance.redis.flushdb()
    await instance.redis.aclose()


async def counts(instance, state="delivered"):
    """Read one milestone counter as an int."""
    raw = await instance.redis.hget(instance.settings.count_table, state)
    return None if raw is None else int(raw)


async def seed(instance, state="delivered", value=1000):
    """Set a milestone counter."""
    await instance.redis.hset(instance.settings.count_table, state, value)


async def add_snapshot(instance, payload: dict, score: float):
    """Write a terminal snapshot at an explicit expiry score."""
    await instance.redis.zadd(instance.settings.terminal_table, {json.dumps(payload): score})


# --------------------------------------------------------------------------
# adjust_counts: score- vs index-addressed ranges
# --------------------------------------------------------------------------


async def test_adjust_counts_leaves_future_snapshots_alone(client):
    """A snapshot that has not aged out must not be applied.

    Regression test against `ZRANGE` call reading rank indexes vs. scores.
    """
    await seed(client, value=1000)
    not_yet_due = (START_OF_DAY_ONE + timedelta(days=1)).timestamp()
    await add_snapshot(client, {"delivered": 1000}, not_yet_due)

    await client.adjust_counts(START_OF_DAY_ONE)

    assert await counts(client) == 1000, "a future-dated snapshot was applied"
    assert await client.redis.zcard(client.settings.terminal_table) == 1


async def test_adjust_counts_retires_aged_out_snapshots(client):
    """A snapshot at or before the cutoff is subtracted and removed."""
    await seed(client, value=1000)
    await add_snapshot(client, {"delivered": 400}, START_OF_DAY_ONE.timestamp())

    await client.adjust_counts(START_OF_DAY_ONE)

    assert await counts(client) == 600
    assert await client.redis.zcard(client.settings.terminal_table) == 0


# --------------------------------------------------------------------------
# terminal_snapshot: once per day
# --------------------------------------------------------------------------


async def test_terminal_snapshot_runs_once_per_day(client, monkeypatch):
    """The second call in a day is a no-op.

    The lock cannot provide this: it is released when the run ends, so a
    `backoffLimit` retry or a manual run would re-apply the adjustment.
    """
    freeze(monkeypatch, DAY_ONE)
    await seed(client, value=1000)

    first = await client.terminal_snapshot()
    assert first == {"delivered": 1000}
    assert await client.redis.zcard(client.settings.terminal_table) == 1

    # More traffic lands, then the job runs again the same day.
    await client.redis.hincrby(client.settings.count_table, "delivered", 100)
    second = await client.terminal_snapshot()

    assert second == {}, "the snapshot ran twice in one day"
    assert await counts(client) == 1100, "counters were adjusted twice"
    assert await client.redis.zcard(client.settings.terminal_table) == 1


async def test_terminal_snapshot_resumes_the_next_day(client, monkeypatch):
    """The next UTC day retires exactly the prior day's snapshot."""
    freeze(monkeypatch, DAY_ONE)
    await seed(client, value=1000)
    await client.terminal_snapshot()

    # Overnight arrivals.
    await client.redis.hincrby(client.settings.count_table, "delivered", 60)

    freeze(monkeypatch, DAY_ONE + timedelta(days=1))
    result = await client.terminal_snapshot()

    assert await counts(client) == 60, "yesterday's backlog was not retired once"
    assert result == {"delivered": 60}
    assert await client.redis.zcard(client.settings.terminal_table) == 1


async def test_marker_is_only_set_after_the_snapshot_lands(client, monkeypatch):
    """A run that dies mid-way is retried, not skipped."""
    freeze(monkeypatch, DAY_ONE)
    await seed(client, value=1000)

    boom = RuntimeError("redis went away")

    async def explode(*args, **kwargs):
        raise boom

    monkeypatch.setattr(client.redis, "zadd", explode)
    with pytest.raises(RuntimeError):
        await client.terminal_snapshot()

    assert await client.redis.get(client.settings.snapshot_marker) is None

    monkeypatch.undo()
    freeze(monkeypatch, DAY_ONE)
    assert await client.terminal_snapshot() == {"delivered": 1000}


# --------------------------------------------------------------------------
# Negative counters
# --------------------------------------------------------------------------


async def test_frequent_runs_do_not_corrupt_counters(client, monkeypatch):
    """Running all day must not drive the counters negative.

    This is the end-to-end guard for the reported failure. Each window applies
    new arrivals minus records reaped by `gc()`, then runs the job. Windows
    where reaping outpaces arrivals are what used to underflow.
    """
    freeze(monkeypatch, DAY_ONE)
    await seed(client, value=1000)

    # (arrivals, reaped) per 10-minute window.
    windows = [(0, 0), (40, 0), (35, 12), (5, 30), (40, 3), (10, 45), (20, 8)]
    for arrivals, reaped in windows:
        await client.redis.hincrby(client.settings.count_table, "delivered", arrivals - reaped)
        await client.terminal_snapshot()
        current = await counts(client)
        assert current >= 0, f"counter went negative: {current}"

    expected = 1000 + sum(a - r for a, r in windows)
    assert await counts(client) == expected
    assert await client.redis.zcard(client.settings.terminal_table) == 1


# --------------------------------------------------------------------------
# Locking
# --------------------------------------------------------------------------


async def test_lock_is_mutually_exclusive(client, monkeypatch):
    """Two runners must not both hold the lock. Regression test
    against when every caller minted a new never-contended key using
    the current timestamp.
    """
    monkeypatch.setenv("BIGTABLE_EMULATOR_HOST", "localhost:8086")
    other = reliability_report.Redis(
        reliability_report.logging.getLogger("autotrack-test-2"), make_settings()
    )
    try:
        assert await client.get_lock() is True
        assert await other.get_lock() is False, "the lock excluded nothing"

        await client.release_lock()
        assert await other.get_lock() is True
        await other.release_lock()
    finally:
        await other.redis.aclose()


# --------------------------------------------------------------------------
# config(): no Redis required
# --------------------------------------------------------------------------


@pytest.fixture
def bare_argv(monkeypatch):
    """Keep `config()`'s `parse_args` away from pytest's own argv."""
    monkeypatch.setattr(sys, "argv", ["reliability_report.py"])


def test_config_coerces_numeric_settings(monkeypatch, bare_argv):
    """Environment-supplied numbers arrive as ints, not strings.

    Without `type=int` these reach `timedelta(days=...)` and `redis.lock(
    timeout=...)` as strings and raise at runtime.
    """
    monkeypatch.setenv("AUTOTRACK_BUCKET_RETENTION_DAYS", "45")
    monkeypatch.setenv("AUTOTRACK_TERM_MAX_RETENTION_DAYS", "3")
    monkeypatch.setenv("AUTOTRACK__LOCK_HOLD_TIME", "900")
    monkeypatch.setenv("AUTOTRACK__LOCK_ACQUISITION_TIME", "20")

    args = reliability_report.config()

    assert args.bucket_retention_days == 45
    assert args.terminal_max_retention_days == 3
    assert args.lock_hold_time == 900
    assert args.lock_acquire_time == 20
    # The operations that used to break on a string.
    assert timedelta(days=args.terminal_max_retention_days).days == 3


def test_config_has_no_default_bucket(monkeypatch, bare_argv):
    """An unset bucket must stay unset.

    It used to default to a bucket that exists in no environment, so reports
    were written nowhere and the misconfiguration was invisible.
    """
    monkeypatch.delenv("AUTOTRACK_REPORT_BUCKET_NAME", raising=False)

    assert reliability_report.config().report_bucket_name is None


def test_config_sets_output_without_a_bucket(monkeypatch, bare_argv):
    """Output formats are populated even with no bucket configured.

    With no bucket the report goes to stdout, which iterates `output`; that
    used to be populated only when a bucket was set, so the path raised.
    """
    monkeypatch.delenv("AUTOTRACK_REPORT_BUCKET_NAME", raising=False)
    monkeypatch.delenv("AUTOTRACK_OUTPUT", raising=False)

    assert reliability_report.config().output == ["md", "json"]


def test_config_filters_unknown_output_formats(monkeypatch, bare_argv):
    """Unknown formats are dropped rather than passed through."""
    monkeypatch.setenv("AUTOTRACK_OUTPUT", "md, csv json")

    assert reliability_report.config().output == ["md", "json"]
