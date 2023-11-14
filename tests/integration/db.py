"""Database Interaction

WebPush Sort Keys
-----------------

Messages for WebPush are stored using a partition key + sort key, originally
the sort key was:

    CHID : Encrypted(UAID: CHID)

The encrypted portion was returned as the Location to the Application Server.
Decrypting it resulted in enough information to create the sort key so that
the message could be deleted and located again.

For WebPush Topic messages, a new scheme was needed since the only way to
locate the prior message is the UAID + CHID + Topic. Using Encryption in
the sort key is therefore not useful since it would change every update.

The sort key scheme for WebPush messages is:

    VERSION : CHID : TOPIC

To ensure updated messages are not deleted, each message will still have an
update-id key/value in its item.

Non-versioned messages are assumed to be original messages from before this
scheme was adopted.

``VERSION`` is a 2-digit 0-padded number, starting at 01 for Topic messages.

"""
from __future__ import absolute_import

import base64
import datetime
import hashlib
import hmac
import os
import random
import threading
import time
import uuid
from functools import wraps
from typing import Any, Callable, TypeVar

import boto3
import botocore
from botocore.exceptions import ClientError

# Autopush Exceptions:


class AutopushException(Exception):
    """Parent Autopush Exception"""


class ItemNotFound(Exception):
    """Signals missing DynamoDB Item data"""


class DynamoDBResource(threading.local):
    def __init__(self, **kwargs):
        conf = kwargs
        if not conf.get("endpoint_url"):
            if os.getenv("AWS_LOCAL_DYNAMODB"):
                conf.update(
                    dict(
                        endpoint_url=os.getenv("AWS_LOCAL_DYNAMODB"),
                        aws_access_key_id="Bogus",
                        aws_secret_access_key="Bogus",
                    )
                )
        # If there is no endpoint URL, we must delete the entry
        if "endpoint_url" in conf and not conf["endpoint_url"]:
            del conf["endpoint_url"]
        if "region_name" in conf:
            del conf["region_name"]
        region = conf.get("region_name", os.getenv("AWS_DEFAULT_REGION", "us-east-1"))
        self.conf = conf
        self._resource = boto3.resource(
            "dynamodb", config=botocore.config.Config(region_name=region), **self.conf
        )

    def __getattr__(self, name):
        return getattr(self._resource, name)

    def get_latest_message_tablenames(
        self, prefix: str = "message", previous: int = 1
    ) -> list[str]:
        """Fetches the name of the last message table"""
        client = self._resource.meta.client
        paginator = client.get_paginator("list_tables")
        tables = []
        for table in paginator.paginate().search(
            "TableNames[?contains(@,'{}')==`true`]|sort(@)[-1]".format(prefix)
        ):
            if table and table.encode().startswith(prefix):
                tables.append(table)
        if not len(tables) or tables[0] is None:
            return [prefix]
        tables.sort()
        return tables[0 - previous :]

    def get_latest_message_tablename(self, prefix: str = "message") -> str:
        """Fetches the name of the last message table"""
        return self.get_latest_message_tablenames(prefix=prefix, previous=1)[0]


class DynamoDBTable(threading.local):
    def __init__(self, ddb_resource: DynamoDBResource, *args, **kwargs) -> None:
        self._table = ddb_resource.Table(*args, **kwargs)

    def __getattr__(self, name):
        return getattr(self._table, name)


# Autopush utils:
def generate_hash(key: bytes, payload: bytes) -> str:
    """Generate a HMAC for the uaid using the secret

    :returns: HMAC hash and the nonce used as a tuple (nonce, hash).
    """
    h = hmac.new(key=key, msg=payload, digestmod=hashlib.sha256)
    return h.hexdigest()


def normalize_id(ident: uuid.UUID | str) -> str:
    if isinstance(ident, uuid.UUID):
        return str(ident)
    try:
        return str(uuid.UUID(ident))
    except ValueError:
        raise ValueError("Invalid UUID")


def base64url_encode(value: bytes | str) -> str:
    if isinstance(value, str):
        value = bytes(value, "utf-8")
    """Encodes an unpadded Base64 URL-encoded string per RFC 7515."""
    return base64.urlsafe_b64encode(value).strip(b"=").decode("utf-8")


# Max DynamoDB record lifespan (~ 30 days)
MAX_EXPIRY = 2592000  # pragma: nocover

# Typing
T = TypeVar("T")  # noqa
TableFunc = Callable[[DynamoDBResource, str, int, int], DynamoDBTable]

key_hash = b""
TRACK_DB_CALLS = False
DB_CALLS = []

MAX_DDB_SESSIONS = 50  # constants.THREAD_POOL_SIZE


def get_month(delta: int = 0) -> datetime.date:
    """Basic helper function to get a datetime.date object iterations months
    ahead/behind of now.
    """
    new = last = datetime.date.today()
    # Move until we hit a new month, this avoids having to manually
    # check year changes as we push forward or backward since the Python
    # timedelta math handles it for us
    for _ in range(abs(delta)):
        while new.month == last.month:
            if delta < 0:
                new -= datetime.timedelta(days=14)
            else:
                new += datetime.timedelta(days=14)
        last = new
    return new


def hasher(uaid: str) -> str:
    """Hashes a key using a key_hash if present"""
    if key_hash:
        return generate_hash(key_hash, bytes(uaid, "utf-8"))
    return uaid


def create_message_table_ddb(
    boto_resource: DynamoDBResource,
    tablename: str,
    read_throughput: int = 5,
    write_throughput: int = 5,
) -> Any:
    """Create a new message table for webpush style message storage"""
    try:
        table = boto_resource.Table(tablename)
        if table.table_status == "ACTIVE":
            return table
    except ClientError as ex:
        if ex.response["Error"]["Code"] != "ResourceNotFoundException":
            raise  # pragma nocover
    table = boto_resource.create_table(
        TableName=tablename,
        KeySchema=[
            {"AttributeName": "uaid", "KeyType": "HASH"},
            {"AttributeName": "chidmessageid", "KeyType": "RANGE"},
        ],
        AttributeDefinitions=[
            {"AttributeName": "uaid", "AttributeType": "S"},
            {"AttributeName": "chidmessageid", "AttributeType": "S"},
        ],
        ProvisionedThroughput={
            "ReadCapacityUnits": read_throughput,
            "WriteCapacityUnits": write_throughput,
        },
    )
    table.meta.client.get_waiter("table_exists").wait(TableName=tablename)
    try:
        table.meta.client.update_time_to_live(
            TableName=tablename,
            TimeToLiveSpecification={
                "Enabled": True,
                "AttributeName": "expiry",
            },
        )
    except ClientError as ex:  # pragma nocover
        if ex.response["Error"]["Code"] != "UnknownOperationException":
            # DynamoDB local library does not yet support TTL
            raise
    return table


def create_router_table_ddb(
    boto_resource: DynamoDBResource,
    tablename: str = "router",
    read_throughput: int = 5,
    write_throughput: int = 5,
) -> Any:
    """Create a new router table

    The last_connect index is a value used to determine the last month a user
    was seen in. To prevent hot-keys on this table during month switchovers the
    key is determined based on the following scheme:

        (YEAR)(MONTH)(DAY)(HOUR)(0001-0010)

    Note that the random key is only between 1-10 at the moment, if the key is
    still too hot during production the random range can be increased at the
    cost of additional queries during GC to locate expired users.
    """
    table = boto_resource.create_table(
        TableName=tablename,
        KeySchema=[{"AttributeName": "uaid", "KeyType": "HASH"}],
        AttributeDefinitions=[{"AttributeName": "uaid", "AttributeType": "S"}],
        ProvisionedThroughput={
            "ReadCapacityUnits": read_throughput,
            "WriteCapacityUnits": write_throughput,
        },
    )
    table.meta.client.get_waiter("table_exists").wait(TableName=tablename)
    # Mobile devices (particularly older ones) do not have expiry and
    # do not check in regularly. We don't know when they expire other than
    # the bridge server failing the UID from their side.
    return table


def _drop_table(boto_resource: DynamoDBResource, tablename: str) -> None:
    try:
        boto_resource.meta.client.delete_table(TableName=tablename)
    except ClientError:
        pass


def _make_table(
    table_func: TableFunc,
    tablename: str,
    read_throughput: int,
    write_throughput: int,
    boto_resource: DynamoDBResource,
) -> DynamoDBTable:
    """Private common function to make a table with a table func"""
    if not boto_resource:
        raise AutopushException("No boto3 resource provided for _make_table")
    if not table_exists(boto_resource, tablename):
        return table_func(boto_resource, tablename, read_throughput, write_throughput)
    else:
        return DynamoDBTable(boto_resource, tablename)


def _expiry(ttl):
    return int(time.time()) + ttl


def get_router_table(
    boto_resource: DynamoDBResource,
    tablename: str = "router",
    read_throughput: int = 5,
    write_throughput: int = 5,
) -> Any:
    """Get the main router table object

    Creates the table if it doesn't already exist, otherwise returns the
    existing table.

    """
    return _make_table(
        create_router_table_ddb,
        tablename,
        read_throughput,
        write_throughput,
        boto_resource=boto_resource,
    )


def track_provisioned(func: Callable[..., T]) -> Callable[..., T]:
    """Tracks provisioned exceptions and increments a metric for them named
    after the function decorated"""

    @wraps(func)
    def wrapper(self, *args, **kwargs):
        if TRACK_DB_CALLS:
            DB_CALLS.append(func.__name__)
        return func(self, *args, **kwargs)

    return wrapper


def has_connected_this_month(item: dict[str, Any]) -> bool:
    """Whether a router item has connected this month"""
    last_connect = item.get("last_connect")
    if not last_connect:
        return False

    today = datetime.datetime.today()
    val = "%s%s" % (today.year, str(today.month).zfill(2))
    return str(last_connect).startswith(val)


def generate_last_connect() -> int:
    """Generate a last_connect

    This intentionally generates a limited set of keys for each month in a
    known sequence. For each month, there's 24 hours * 10 random numbers for
    a total of 240 keys per month depending on when the user migrates forward.

    """
    today = datetime.datetime.today()
    val = "".join(
        [
            str(today.year),
            str(today.month).zfill(2),
            str(today.hour).zfill(2),
            str(random.randint(0, 10)).zfill(4),
        ]
    )
    return int(val)


def table_exists(boto_resource: DynamoDBResource, tablename: str) -> bool:
    """Determine if the specified Table exists"""
    try:
        return boto_resource.Table(tablename).table_status in ["CREATING", "UPDATING", "ACTIVE"]
    except ClientError:
        return False
