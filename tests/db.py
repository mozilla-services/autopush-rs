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
import hmac
import hashlib
import os
import random
import threading
import time
import uuid
from functools import wraps

from attr import attrs, attrib, Factory

import boto3
from boto3.resources.base import ServiceResource  # noqa
from boto3.dynamodb.conditions import Key
from botocore.exceptions import ClientError

from typing import (  # noqa
    TYPE_CHECKING,
    Any,
    Callable,
    Dict,
    Generator,
    Iterable,
    List,
    Optional,
    Set,
    TypeVar,
    Tuple,
    Union,
)

# Autopush Exceptions:


class AutopushException(Exception):
    """Parent Autopush Exception"""


class ItemNotFound(Exception):
    """Signals missing DynamoDB Item data"""


# Autopush utils:
def generate_hash(key: bytes, payload: bytes) -> str:
    """Generate a HMAC for the uaid using the secret

    :returns: HMAC hash and the nonce used as a tuple (nonce, hash).

    """
    h = hmac.new(key=key, msg=payload, digestmod=hashlib.sha256)
    return h.hexdigest()


def normalize_id(ident):
    # type: (Union[uuid.UUID, str]) -> str
    if isinstance(ident, uuid.UUID):
        return str(ident)
    try:
        return str(uuid.UUID(ident))
    except ValueError:
        raise ValueError("Invalid UUID")


def base64url_encode(value):
    if isinstance(value, str):
        value = bytes(value, "utf-8")
    # type: (bytes) -> str
    """Encodes an unpadded Base64 URL-encoded string per RFC 7515."""
    return base64.urlsafe_b64encode(value).strip(b"=").decode("utf-8")


@attrs(slots=True)
class WebPushNotification(object):
    """WebPush Notification

    This object centralizes all logic involving the addressing of a single
    WebPush Notification.

    message_id serves a complex purpose. It's returned as the Location header
    value so that an application server may delete the message. It's used as
    part of the non-versioned sort-key. Due to this, its an encrypted value
    that contains the necessary information to derive the location of this
    precise message in the appropriate message table.

    """

    uaid: uuid.UUID = attrib()
    channel_id: uuid.UUID = attrib()
    ttl: int = attrib()  # type: int
    data: Optional[str] = attrib(default=None)
    headers: Optional[Dict[str, str]] = attrib(default=None)
    timestamp: int = attrib(default=Factory(lambda: int(time.time())))
    sortkey_timestamp: Optional[int] = attrib(default=None)
    topic: Optional[str] = attrib(default=None)
    source: Optional[str] = attrib(default="Direct")

    message_id: str = attrib(default=None)

    # Not an alias for message_id, for backwards compat and cases where an old
    # message with any update_id should be removed.
    update_id: str = attrib(default=None)

    # Whether this notification should follow legacy non-topic rules
    legacy: bool = attrib(default=False)

    @staticmethod
    def parse_sort_key(sort_key: str) -> Dict[str, Any]:
        # type: (str) -> Dict[str, Any]
        """Parse the sort key from the database"""
        topic = None
        sortkey_timestamp = None
        message_id = None
        if sort_key.startswith("01:"):
            api_ver, channel_id, topic = sort_key.split(":")
        elif sort_key.startswith("02:"):
            api_ver, raw_sortkey, channel_id = sort_key.split(":")
            sortkey_timestamp = int(raw_sortkey)
        else:
            channel_id, message_id = sort_key.split(":")
            api_ver = "00"
        return dict(
            api_ver=api_ver,
            channel_id=channel_id,
            topic=topic,
            message_id=message_id,
            sortkey_timestamp=sortkey_timestamp,
        )

    @classmethod
    def from_message_table(
        cls, uaid: str, item: Dict[str, (str | Dict[str, str])]
    ):
        # type: (uuid.UUID, Dict[str, Any]) -> WebPushNotification
        """Create a WebPushNotification from a message table item"""
        key_info = cls.parse_sort_key(item["chidmessageid"])
        if key_info["api_ver"] in ["01", "02"]:
            key_info["message_id"] = item["updateid"]
        notif = cls(
            uaid=uuid.UUID(uaid),
            channel_id=uuid.UUID(key_info["channel_id"]),
            data=item.get("data"),
            headers=item.get("headers"),
            ttl=item.get("ttl", 0),
            topic=key_info.get("topic"),
            message_id=key_info["message_id"],
            update_id=item.get("updateid"),
            timestamp=item.get("timestamp"),
            sortkey_timestamp=key_info.get("sortkey_timestamp"),
            source="Stored",
        )
        # Ensure we generate the sort-key properly for legacy messges
        if key_info["api_ver"] == "00":
            notif.legacy = True

        return notif


# Max DynamoDB record lifespan (~ 30 days)
MAX_EXPIRY = 2592000  # pragma: nocover

# Typing
T = TypeVar("T")  # noqa
TableFunc = Callable[[str, int, int, ServiceResource], Any]

key_hash = b""
TRACK_DB_CALLS = False
DB_CALLS = []

MAX_DDB_SESSIONS = 50  # constants.THREAD_POOL_SIZE


def get_month(delta=0):
    # type: (int) -> datetime.date
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


def hasher(uaid):
    # type: (str) -> str
    """Hashes a key using a key_hash if present"""
    if key_hash:
        return generate_hash(key_hash, bytes(uaid, "utf-8"))
    return uaid


def create_message_table_ddb(
    tablename,  # type: str
    read_throughput=5,  # type: int
    write_throughput=5,  # type: int
    boto_resource=None,  # type: DynamoDBResource
):
    # type: (...) -> Any  # noqa
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
    tablename="router",
    read_throughput=5,
    write_throughput=5,
    boto_resource=None,
):
    # type: (str, int, int, DynamoDBResource) -> Any
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
        AttributeDefinitions=[
            {"AttributeName": "uaid", "AttributeType": "S"}
        ],
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


def _drop_table(tablename, boto_resource):
    # type: (str, DynamoDBResource) -> None
    try:
        boto_resource.meta.client.delete_table(TableName=tablename)
    except ClientError:  # pragma nocover
        pass


def _make_table(
    table_func,  # type: TableFunc
    tablename,  # type: str
    read_throughput,  # type: int
    write_throughput,  # type: int
    boto_resource,  # type: DynamoDBResource
):
    # type (...) -> DynamoDBTable
    """Private common function to make a table with a table func"""
    if not boto_resource:
        raise AutopushException("No boto3 resource provided for _make_table")
    if not table_exists(tablename, boto_resource):
        return table_func(
            tablename, read_throughput, write_throughput, boto_resource
        )
    else:
        return DynamoDBTable(boto_resource, tablename)


def _expiry(ttl):
    return int(time.time()) + ttl


def get_router_table(
    tablename="router",
    read_throughput=5,
    write_throughput=5,
    boto_resource=None,
):
    # type: (str, int, int, DynamoDBResource) -> Any
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


def track_provisioned(func):
    # type: (Callable[..., T]) -> Callable[..., T]
    """Tracks provisioned exceptions and increments a metric for them named
    after the function decorated"""

    @wraps(func)
    def wrapper(self, *args, **kwargs):
        if TRACK_DB_CALLS:
            DB_CALLS.append(func.__name__)
        return func(self, *args, **kwargs)

    return wrapper


def has_connected_this_month(item):
    # type: (Dict[str, Any]) -> bool
    """Whether or not a router item has connected this month"""
    last_connect = item.get("last_connect")
    if not last_connect:
        return False

    today = datetime.datetime.today()
    val = "%s%s" % (today.year, str(today.month).zfill(2))
    return str(last_connect).startswith(val)


def generate_last_connect():
    # type: () -> int
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


def table_exists(tablename, boto_resource=None):
    # type: (str, DynamoDBResource) -> bool
    """Determine if the specified Table exists"""
    try:
        return boto_resource.Table(tablename).table_status in [
            "CREATING",
            "UPDATING",
            "ACTIVE",
        ]
    except ClientError:
        return False


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
        self.conf = conf
        self._resource = boto3.resource("dynamodb", **self.conf)

    def __getattr__(self, name):
        return getattr(self._resource, name)

    def get_latest_message_tablenames(self, prefix="message", previous=1):
        # # type: (Optional[str], int) -> [str]  # noqa
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
        return tables[0 - previous:]

    def get_latest_message_tablename(self, prefix="message"):
        # type: (Optional[str]) -> str  # noqa
        """Fetches the name of the last message table"""
        return self.get_latest_message_tablenames(prefix=prefix, previous=1)[
            0
        ]


class DynamoDBTable(threading.local):
    def __init__(self, ddb_resource, *args, **kwargs):
        # type: (DynamoDBResource, *Any, **Any) -> None
        self._table = ddb_resource.Table(*args, **kwargs)

    def __getattr__(self, name):
        return getattr(self._table, name)


class Message(object):
    """Create a Message table abstraction on top of a DynamoDB Table object"""

    def __init__(self, tablename, boto_resource=None, max_ttl=MAX_EXPIRY):
        # type: (str, DynamoDBResource, int) -> None
        """Create a new Message object

        :param tablename: name of the table.
        :param boto_resource: DynamoDBResource for thread

        """
        self._max_ttl = max_ttl
        self.resource = boto_resource
        self.table = DynamoDBTable(self.resource, tablename)
        self.tablename = tablename

    def table_status(self):
        return self.table.table_status

    @track_provisioned
    def register_channel(self, uaid, channel_id, ttl=None):
        # type: (str, str, int) -> bool
        """Register a channel for a given uaid"""
        # Generate our update expression
        if ttl is None:
            ttl = self._max_ttl
        # Create/Append to list of channel_ids
        self.table.update_item(
            Key={
                "uaid": hasher(uaid),
                "chidmessageid": " ",
            },
            UpdateExpression="ADD chids :channel_id SET expiry = :expiry",
            ExpressionAttributeValues={
                ":channel_id": set([normalize_id(channel_id)]),
                ":expiry": _expiry(ttl),
            },
        )
        return True

    @track_provisioned
    def unregister_channel(self, uaid, channel_id, **kwargs):
        # type: (str, str, **str) -> bool
        """Remove a channel registration for a given uaid"""
        chid = normalize_id(channel_id)

        response = self.table.update_item(
            Key={
                "uaid": hasher(uaid),
                "chidmessageid": " ",
            },
            UpdateExpression="DELETE chids :channel_id SET expiry = :expiry",
            ExpressionAttributeValues={
                ":channel_id": set([chid]),
                ":expiry": _expiry(self._max_ttl),
            },
            ReturnValues="UPDATED_OLD",
        )
        chids = response.get("Attributes", {}).get("chids", {})
        if chids:
            try:
                return chid in chids
            except (TypeError, AttributeError):  # pragma: nocover
                pass
        # if, for some reason, there are no chids defined, return False.
        return False

    @track_provisioned
    def all_channels(self, uaid):
        # type: (str) -> Tuple[bool, Set[str]]
        """Retrieve a list of all channels for a given uaid"""

        # Note: This only returns the chids associated with the UAID.
        # Functions that call store_message() would be required to
        # update that list as well using register_channel()
        result = self.table.get_item(
            Key={
                "uaid": hasher(uuid.UUID(uaid).hex),
                "chidmessageid": " ",
            },
            ConsistentRead=True,
        )
        if result["ResponseMetadata"]["HTTPStatusCode"] != 200:
            return False, set([])
        if "Item" not in result:
            return False, set([])
        return True, result["Item"].get("chids", set([]))

    @track_provisioned
    def save_channels(self, uaid, channels):
        # type: (str, Set[str]) -> None
        """Save out a set of channels"""
        self.table.put_item(
            Item={
                "uaid": hasher(uaid),
                "chidmessageid": " ",
                "chids": channels,
                "expiry": _expiry(self._max_ttl),
            },
        )

    @track_provisioned
    def store_message(self, notification):
        # type: (WebPushNotification) -> None
        """Stores a WebPushNotification in the message table"""
        item = dict(
            uaid=hasher(notification.uaid.hex),
            chidmessageid=notification.sort_key,
            headers=notification.headers,
            ttl=notification.ttl,
            timestamp=notification.timestamp,
            updateid=notification.update_id,
            expiry=_expiry(min(notification.ttl or 0, self._max_ttl)),
        )
        if notification.data:
            item["data"] = notification.data
        self.table.put_item(Item=item)

    @track_provisioned
    def delete_message(self, notification):
        # type: (WebPushNotification) -> bool
        """Deletes a specific message"""
        if notification.update_id:
            try:
                self.table.delete_item(
                    Key={
                        "uaid": hasher(notification.uaid.hex),
                        "chidmessageid": notification.sort_key,
                    },
                    Expected={
                        "updateid": {
                            "Exists": True,
                            "Value": notification.update_id,
                        }
                    },
                )
            except ClientError:
                return False
        else:
            self.table.delete_item(
                Key={
                    "uaid": hasher(notification.uaid.hex),
                    "chidmessageid": notification.sort_key,
                }
            )
        return True

    @track_provisioned
    def fetch_messages(
        self,
        uaid,  # type: uuid.UUID
        limit=10,  # type: int
    ):
        # type: (...) -> Tuple[Optional[int], List[WebPushNotification]]
        """Fetches messages for a uaid

        :returns: A tuple of the last timestamp to read for timestamped
                  messages and the list of non-timestamped messages.

        """
        # Eagerly fetches all results in the result set.
        response = self.table.query(
            KeyConditionExpression=(
                Key("uaid").eq(hasher(uaid.hex))
                & Key("chidmessageid").lt("02")
            ),
            ConsistentRead=True,
            Limit=limit,
        )
        results = list(response["Items"])
        # First extract the position if applicable, slightly higher than
        # 01: to ensure we don't load any 01 remainders that didn't get
        # deleted yet
        last_position = None
        if results:
            # Ensure we return an int, as boto2 can return Decimals
            if results[0].get("current_timestamp"):
                last_position = int(results[0]["current_timestamp"])

        return last_position, [
            WebPushNotification.from_message_table(uaid, x)
            for x in results[1:]
        ]

    @track_provisioned
    def fetch_timestamp_messages(
        self,
        uaid,  # type: uuid.UUID
        timestamp=None,  # type: Optional[Union[int, str]]
        limit=10,  # type: int
    ):
        # type: (...) -> Tuple[Optional[int], List[WebPushNotification]]
        """Fetches timestamped messages for a uaid

        Note that legacy messages start with a hex UUID, so they may be mixed
        in with timestamp messages beginning with 02. As such we only move our
        last_position forward to the last timestamped message.

        :returns: A tuple of the last timestamp to read and the list of
                  timestamped messages.

        """
        # Turn the timestamp into a proper sort key
        if timestamp:
            sortkey = "02:{timestamp}:z".format(timestamp=timestamp)
        else:
            sortkey = "01;"

        response = self.table.query(
            KeyConditionExpression=(
                Key("uaid").eq(hasher(uaid.hex))
                & Key("chidmessageid").gt(sortkey)
            ),
            ConsistentRead=True,
            Limit=limit,
        )
        notifs = [
            WebPushNotification.from_message_table(uaid, x)
            for x in response.get("Items")
        ]
        ts_notifs = [x for x in notifs if x.sortkey_timestamp]
        last_position = None
        if ts_notifs:
            last_position = ts_notifs[-1].sortkey_timestamp
        return last_position, notifs

    @track_provisioned
    def update_last_message_read(self, uaid, timestamp):
        # type: (uuid.UUID, int) -> bool
        """Update the last read timestamp for a user"""
        expr = "SET current_timestamp=:timestamp, expiry=:expiry"
        expr_values = {
            ":timestamp": timestamp,
            ":expiry": _expiry(self._max_ttl),
        }
        self.table.update_item(
            Key={"uaid": hasher(uaid.hex), "chidmessageid": " "},
            UpdateExpression=expr,
            ExpressionAttributeValues=expr_values,
        )
        return True
