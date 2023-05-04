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

from attr import (
    asdict,
    attrs,
    attrib,
    Factory,
    Attribute)

import boto3
import botocore
from boto3.resources.base import ServiceResource  # noqa
from boto3.dynamodb.conditions import Key
from boto3.exceptions import Boto3Error
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
# from twisted.internet.defer import inlineCallbacks, returnValue
# from twisted.internet.threads import deferToThread

# import autopush.metrics
# from autopush import constants
# from autopush.exceptions import AutopushException, ItemNotFound
# from autopush.metrics import IMetrics  # noqa
# from autopush.utils import (
#     generate_hash,
#     normalize_id,
#     WebPushNotification,
# )

# Autopush Exceptions:

class AutopushException(Exception):
    """Parent Autopush Exception"""

class ItemNotFound(Exception):
    """Signals missing DynamoDB Item data"""

# Autopush utils:
def generate_hash(key, payload):
    # type: (str, str) -> str
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

def base64url_encode(string):
    # type: (str) -> str
    """Encodes an unpadded Base64 URL-encoded string per RFC 7515."""
    return base64.urlsafe_b64encode(string).strip('=')


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
    uaid = attrib()  # type: uuid.UUID
    channel_id = attrib()  # type: uuid.UUID
    ttl = attrib()  # type: int
    data = attrib(default=None)  # type: Optional[str]
    headers = attrib(default=None)  # type: Optional[Dict[str, str]]
    timestamp = attrib(default=Factory(lambda: int(time.time())))  # type: int
    sortkey_timestamp = attrib(default=None)  # type: Optional[int]
    topic = attrib(default=None)  # type: Optional[str]
    source = attrib(default="Direct")  # type: Optional[str]

    message_id = attrib(default=None)  # type: str

    # Not an alias for message_id, for backwards compat and cases where an old
    # message with any update_id should be removed.
    update_id = attrib(default=None)  # type: str

    # Whether this notification should follow legacy non-topic rules
    legacy = attrib(default=False)  # type: bool

    @staticmethod
    def parse_sort_key(sort_key):
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
        return dict(api_ver=api_ver, channel_id=channel_id,
                    topic=topic, message_id=message_id,
                    sortkey_timestamp=sortkey_timestamp)


    @classmethod
    def from_message_table(cls, uaid, item):
        # type: (uuid.UUID, Dict[str, Any]) -> WebPushNotification
        """Create a WebPushNotification from a message table item"""
        key_info = cls.parse_sort_key(item["chidmessageid"])
        if key_info["api_ver"] in ["01", "02"]:
            key_info["message_id"] = item["updateid"]
        notif = cls(
            uaid=uaid,
            channel_id=uuid.UUID(key_info["channel_id"]),
            data=item.get("data"),
            headers=item.get("headers"),
            ttl=item.get("ttl", 0),
            topic=key_info.get("topic"),
            message_id=key_info["message_id"],
            update_id=item.get("updateid"),
            timestamp=item.get("timestamp"),
            sortkey_timestamp=key_info.get("sortkey_timestamp"),
            source="Stored"
        )
        # Ensure we generate the sort-key properly for legacy messges
        if key_info["api_ver"] == "00":
            notif.legacy = True

        return notif


# if TYPE_CHECKING:  # pragma: nocover
#    from autopush.config import AutopushConfig, DDBTableConfig  # noqa


# Max DynamoDB record lifespan (~ 30 days)
MAX_EXPIRY = 2592000  # pragma: nocover

# Typing
T = TypeVar('T')  # noqa
TableFunc = Callable[[str, int, int, ServiceResource], Any]

key_hash = ""
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
        return generate_hash(key_hash, uaid)
    return uaid


def make_rotating_tablename(prefix, delta=0, date=None):
    # type: (str, int, Optional[datetime.date]) -> str
    """Creates a tablename for table rotation based on a prefix with a given
    month delta."""
    if not date:
        date = get_month(delta=delta)
    return "{}_{:04d}_{:02d}".format(prefix, date.year, date.month)


def create_rotating_message_table(
        prefix="message",     # type: str
        delta=0,              # type: int
        date=None,            # type: Optional[datetime.date]
        read_throughput=5,    # type: int
        write_throughput=5,   # type: int
        boto_resource=None    # type: DynamoDBResource
        ):
    # type: (...) -> Any  # noqa
    """Create a new message table for webpush style message storage with a
        rotating name.

        """

    tablename = make_rotating_tablename(prefix, delta, date)
    return create_message_table(
        tablename=tablename,
        read_throughput=read_throughput,
        write_throughput=write_throughput,
        boto_resource=boto_resource
    )


def create_message_table(
        tablename,  # type: str
        read_throughput=5,  # type: int
        write_throughput=5,  # type: int
        boto_resource=None,  # type: DynamoDBResource
        ):
    # type: (...) -> Any  # noqa
    """Create a new message table for webpush style message storage"""
    try:
        table = boto_resource.Table(tablename)
        if table.table_status == 'ACTIVE':
            return table
    except ClientError as ex:
        if ex.response['Error']['Code'] != 'ResourceNotFoundException':
            raise  # pragma nocover
    table = boto_resource.create_table(
        TableName=tablename,
        KeySchema=[
            {
                'AttributeName': 'uaid',
                'KeyType': 'HASH'
            },
            {
                'AttributeName': 'chidmessageid',
                'KeyType': 'RANGE'
            }],
        AttributeDefinitions=[
            {
                'AttributeName': 'uaid',
                'AttributeType': 'S'
            },
            {
                'AttributeName': 'chidmessageid',
                'AttributeType': 'S'
            }],
        ProvisionedThroughput={
            'ReadCapacityUnits': read_throughput,
            'WriteCapacityUnits': write_throughput
        })
    table.meta.client.get_waiter('table_exists').wait(
        TableName=tablename)
    try:
        table.meta.client.update_time_to_live(
            TableName=tablename,
            TimeToLiveSpecification={
                'Enabled': True,
                'AttributeName': 'expiry'
            }
        )
    except ClientError as ex:  # pragma nocover
        if ex.response['Error']['Code'] != 'UnknownOperationException':
            # DynamoDB local library does not yet support TTL
            raise
    return table


def get_rotating_message_tablename(
        prefix="message",            # type: str
        delta=0,                     # type: int
        date=None,                   # type: Optional[datetime.date]
        message_read_throughput=5,   # type: int
        message_write_throughput=5,  # type: int
        boto_resource=None           # type: DynamoDBResource
        ):
    # type: (...) -> str  # noqa
    """Gets the message table for the current month."""
    tablename = make_rotating_tablename(prefix, delta, date)
    if not table_exists(tablename, boto_resource=boto_resource):
        create_rotating_message_table(
            prefix=prefix, delta=delta, date=date,
            read_throughput=message_read_throughput,
            write_throughput=message_write_throughput,
            boto_resource=boto_resource
        )
        return tablename
    else:
        return tablename


def create_router_table(tablename="router", read_throughput=5,
                        write_throughput=5,
                        boto_resource=None):
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
        KeySchema=[
            {
                'AttributeName': 'uaid',
                'KeyType': 'HASH'
            }
        ],
        AttributeDefinitions=[
            {
                'AttributeName': 'uaid',
                'AttributeType': 'S'
            }],
        ProvisionedThroughput={
            'ReadCapacityUnits': read_throughput,
            'WriteCapacityUnits': write_throughput,
        }
    )
    table.meta.client.get_waiter('table_exists').wait(
        TableName=tablename)
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
        table_func,        # type: TableFunc
        tablename,         # type: str
        read_throughput,   # type: int
        write_throughput,  # type: int
        boto_resource      # type: DynamoDBResource
        ):
    # type (...) -> DynamoDBTable
    """Private common function to make a table with a table func"""
    if not boto_resource:
        raise AutopushException("No boto3 resource provided for _make_table")
    if not table_exists(tablename, boto_resource):
        return table_func(tablename, read_throughput, write_throughput,
                          boto_resource)
    else:
        return DynamoDBTable(boto_resource, tablename)


def _expiry(ttl):
    return int(time.time()) + ttl


def get_router_table(tablename="router", read_throughput=5,
                     write_throughput=5, boto_resource=None):
    # type: (str, int, int, DynamoDBResource) -> Any
    """Get the main router table object

    Creates the table if it doesn't already exist, otherwise returns the
    existing table.

    """
    return _make_table(create_router_table, tablename, read_throughput,
                       write_throughput, boto_resource=boto_resource)


def preflight_check(message, router, uaid="deadbeef00000000deadbeef00000000"):
    # type: (Message, Router, str) -> None
    """Performs a pre-flight check of the router/message to ensure
    appropriate permissions for operation.

    Failure to run correctly will raise an exception.

    """
    # Verify tables are ready for use if they just got created
    ready = False
    while not ready:
        tbl_status = [x.table_status() for x in [message, router]]
        ready = all([status == "ACTIVE" for status in tbl_status])
        if not ready:
            time.sleep(1)

    # Use a distinct UAID so it doesn't interfere with metrics
    uaid = uuid.UUID(uaid)
    chid = uuid.uuid4()
    message_id = str(uuid.uuid4())
    node_id = "mynode:2020"
    connected_at = 0
    notif = WebPushNotification(
        uaid=uaid,
        channel_id=chid,
        update_id=message_id,
        message_id=message_id,
        ttl=60,
    )
    # Store a notification, fetch it, delete it
    message.store_message(notif)
    assert message.delete_message(notif)

    # Store a router entry, fetch it, delete it
    router.register_user(dict(uaid=uaid.hex, node_id=node_id,
                              connected_at=connected_at,
                              current_month=datetime.date.today().month,
                              router_type="webpush"))
    item = router.get_uaid(uaid.hex)
    assert item.get("node_id") == node_id
    # Clean up the preflight data.
    router.clear_node(item)
    router.drop_user(uaid.hex)


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
    val = "".join([
        str(today.year),
        str(today.month).zfill(2),
        str(today.hour).zfill(2),
        str(random.randint(0, 10)).zfill(4),
    ])
    return int(val)


def table_exists(tablename, boto_resource=None):
    # type: (str, DynamoDBResource) -> bool
    """Determine if the specified Table exists"""
    try:
        return boto_resource.Table(tablename).table_status in [
            'CREATING', 'UPDATING', 'ACTIVE']
    except ClientError:
        return False


class DynamoDBResource(threading.local):
    def __init__(self, **kwargs):
        conf = kwargs
        if not conf.get("endpoint_url"):
            if os.getenv("AWS_LOCAL_DYNAMODB"):
                conf.update(dict(
                    endpoint_url=os.getenv("AWS_LOCAL_DYNAMODB"),
                    aws_access_key_id="Bogus",
                    aws_secret_access_key="Bogus"))
        # If there is no endpoint URL, we must delete the entry
        if "endpoint_url" in conf and not conf["endpoint_url"]:
            del(conf["endpoint_url"])
        region = conf.get("region_name",
                          os.getenv("AWS_DEFAULT_REGION", "us-east-1"))
        if "region_name" in conf:
            del(conf["region_name"])
        self.conf = conf
        self._resource = boto3.resource(
            "dynamodb",
            config=botocore.config.Config(region_name=region),
            **self.conf)

    def __getattr__(self, name):
        return getattr(self._resource, name)

    def get_latest_message_tablenames(self, prefix="message", previous=1):
        # # type: (Optional[str], int) -> [str]  # noqa
        """Fetches the name of the last message table"""
        client = self._resource.meta.client
        paginator = client.get_paginator("list_tables")
        tables = []
        for table in paginator.paginate().search(
                "TableNames[?contains(@,'{}')==`true`]|sort(@)[-1]".format(
                    prefix)):
            if table and table.encode().startswith(prefix):
                tables.append(table)
        if not len(tables) or tables[0] is None:
            return [prefix]
        tables.sort()
        return tables[0-previous:]

    def get_latest_message_tablename(self, prefix="message"):
        # type: (Optional[str]) -> str  # noqa
        """Fetches the name of the last message table"""
        return self.get_latest_message_tablenames(
                prefix=prefix,
                previous=1
            )[0]


class DynamoDBTable(threading.local):
    def __init__(self, ddb_resource, *args, **kwargs):
        # type: (DynamoDBResource, *Any, **Any) -> None
        self._table = ddb_resource.Table(*args, **kwargs)

    def __getattr__(self, name):
        return getattr(self._table, name)


class Message(object):
    """Create a Message table abstraction on top of a DynamoDB Table object"""
    def __init__(self, tablename, boto_resource=None,
                 max_ttl=MAX_EXPIRY):
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
                'uaid': hasher(uaid),
                'chidmessageid': ' ',
            },
            UpdateExpression='ADD chids :channel_id SET expiry = :expiry',
            ExpressionAttributeValues={
                ":channel_id": set([normalize_id(channel_id)]),
                ":expiry": _expiry(ttl)
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
                'uaid': hasher(uaid),
                'chidmessageid': ' ',
            },
            UpdateExpression="DELETE chids :channel_id SET expiry = :expiry",
            ExpressionAttributeValues={
                ":channel_id": set([chid]),
                ":expiry": _expiry(self._max_ttl)
            },
            ReturnValues="UPDATED_OLD",
        )
        chids = response.get('Attributes', {}).get('chids', {})
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
                'uaid': hasher(uuid.UUID(uaid).hex),
                'chidmessageid': ' ',
            },
            ConsistentRead=True
        )
        if result['ResponseMetadata']['HTTPStatusCode'] != 200:
            return False, set([])
        if 'Item' not in result:
            return False, set([])
        return True, result['Item'].get("chids", set([]))

    @track_provisioned
    def save_channels(self, uaid, channels):
        # type: (str, Set[str]) -> None
        """Save out a set of channels"""
        self.table.put_item(
            Item={
                'uaid': hasher(uaid),
                'chidmessageid': ' ',
                'chids': channels,
                'expiry': _expiry(self._max_ttl),
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
            expiry=_expiry(min(
                notification.ttl or 0,
                self._max_ttl))
        )
        if notification.data:
            item['data'] = notification.data
        self.table.put_item(Item=item)

    @track_provisioned
    def delete_message(self, notification):
        # type: (WebPushNotification) -> bool
        """Deletes a specific message"""
        if notification.update_id:
            try:
                self.table.delete_item(
                    Key={
                        'uaid': hasher(notification.uaid.hex),
                        'chidmessageid': notification.sort_key
                    },
                    Expected={
                        'updateid': {
                            'Exists': True,
                            'Value': notification.update_id
                            }
                    })
            except ClientError:
                return False
        else:
            self.table.delete_item(
                Key={
                    'uaid': hasher(notification.uaid.hex),
                    'chidmessageid': notification.sort_key,
                })
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
            KeyConditionExpression=(Key("uaid").eq(hasher(uaid.hex))
                                    & Key('chidmessageid').lt('02')),
            ConsistentRead=True,
            Limit=limit,
        )
        results = list(response['Items'])
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
            KeyConditionExpression=(Key('uaid').eq(hasher(uaid.hex))
                                    & Key('chidmessageid').gt(sortkey)),
            ConsistentRead=True,
            Limit=limit
        )
        notifs = [
            WebPushNotification.from_message_table(uaid, x) for x in
            response.get("Items")
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
        expr_values = {":timestamp": timestamp,
                       ":expiry": _expiry(self._max_ttl)}
        self.table.update_item(
            Key={
                "uaid": hasher(uaid.hex),
                "chidmessageid": " "
            },
            UpdateExpression=expr,
            ExpressionAttributeValues=expr_values,
        )
        return True


class Router(object):
    """Create a Router table abstraction on top of a DynamoDB Table object"""
    def __init__(self, conf, metrics, resource=None):
        # # type: (DDBTableConfig, IMetrics, DynamoDBResource) -> None
        """Create a new Router object

        :param conf: configuration data.
        :param metrics: Metrics object that implements the
                        :class:`autopush.metrics.IMetrics` interface.
        :param resource: Boto3 resource handle

        """
        self.conf = conf
        self.metrics = metrics
        self._cached_table = None
        self._resource = resource or DynamoDBResource(**asdict(self.conf))
        self.table = get_router_table(
            tablename=self.conf.tablename,
            boto_resource=self._resource
        )

    def table_status(self):
        return self.table.table_status

    def get_uaid(self, uaid):
        # type: (str) -> Dict[str, Any]
        """Get the database record for the UAID

        :raises:
            :exc:`ItemNotFound` if there is no record for this UAID.
            :exc:`ProvisionedThroughputExceededException` if dynamodb table
            exceeds throughput.

        """
        try:
            item = self.table.get_item(
                Key={
                    'uaid': hasher(uaid)
                },
                ConsistentRead=True,
            )

            if item.get('ResponseMetadata').get('HTTPStatusCode') != 200:
                raise ItemNotFound('uaid not found')
            item = item.get('Item')
            if item is None:
                raise ItemNotFound("uaid not found")
            if item.keys() == ['uaid']:
                # Incomplete record, drop it.
                self.drop_user(uaid)
                raise ItemNotFound("uaid not found")
            # Mobile users do not check in after initial registration.
            # DO NOT EXPIRE THEM.
            return item
        except Boto3Error:  # pragma: nocover
            # We trap JSONResponseError because Moto returns text instead of
            # JSON when looking up values in empty tables. We re-throw the
            # correct ItemNotFound exception
            raise ItemNotFound("uaid not found")

    @track_provisioned
    def register_user(self, data):
        # type: (Dict[str, Any]) -> Tuple[bool, Dict[str, Any]]
        """Register this user

        If a record exists with a newer ``connected_at``, then the user will
        not be registered.

        :returns: Whether the user was registered or not.
        :raises:
            :exc:`ProvisionedThroughputExceededException` if dynamodb table
            exceeds throughput.

        """
        # Fetch a senderid for this user
        db_key = {"uaid": hasher(data["uaid"])}
        del data["uaid"]
        if "router_type" not in data or "connected_at" not in data:
            # Not specifying these values will generate an exception in
            # AWS.
            raise AutopushException("data is missing router_type "
                                    "or connected_at")
        # Generate our update expression
        expr = "SET " + ", ".join(["%s=:%s" % (x, x) for x in data.keys()])
        expr_values = {":%s" % k: v for k, v in data.items()}
        try:
            cond = """(
                attribute_not_exists(router_type) or
                (router_type = :router_type)
            ) and (
                attribute_not_exists(node_id) or
                (connected_at < :connected_at)
            )"""
            result = self.table.update_item(
                Key=db_key,
                UpdateExpression=expr,
                ConditionExpression=cond,
                ExpressionAttributeValues=expr_values,
                ReturnValues="ALL_OLD",
            )
            if "Attributes" in result:
                r = {}
                for key, value in result["Attributes"].items():
                    try:
                        r[key] = self.table._dynamizer.decode(value)
                    except (TypeError, AttributeError):  # pragma: nocover
                        # Included for safety as moto has occasionally made
                        # this not work
                        r[key] = value
                result = r
            return (True, result)
        except ClientError as ex:
            # ClientErrors are generated by a factory, and while they have
            # a class, it's dynamically generated.
            if ex.response['Error']['Code'] == \
                    'ConditionalCheckFailedException':
                return (False, {})
            raise

    @track_provisioned
    def drop_user(self, uaid):
        # type: (str) -> bool
        """Drops a user record"""
        # The following hack ensures that only uaids that exist and are
        # deleted return true.
        try:
            item = self.table.get_item(
                Key={
                    'uaid': hasher(uaid)
                },
                ConsistentRead=True,
            )
            if 'Item' not in item:
                return False
        except ClientError:  # pragma nocover
            pass
        result = self.table.delete_item(
            Key={'uaid': hasher(uaid)})
        return result['ResponseMetadata']['HTTPStatusCode'] == 200

    @track_provisioned
    def _update_last_connect(self, uaid, last_connect):
        self.table.update_item(
            Key={"uaid": hasher(uaid)},
            UpdateExpression="SET last_connect=:last_connect",
            ExpressionAttributeValues={":last_connect": last_connect}
        )

    @track_provisioned
    def update_message_month(self, uaid, month):
        # type: (str, str) -> bool
        """Update the route tables current_message_month

        Note that we also update the last_connect at this point since webpush
        users when connecting will always call this once that month. The
        current_timestamp is also reset as a new month has no last read
        timestamp.

        """
        db_key = {"uaid": hasher(uaid)}
        expr = ("SET current_month=:curmonth, last_connect=:last_connect")
        expr_values = {":curmonth": month,
                       ":last_connect": generate_last_connect(),
                       }
        self.table.update_item(
            Key=db_key,
            UpdateExpression=expr,
            ExpressionAttributeValues=expr_values,
        )
        return True

    @track_provisioned
    def clear_node(self, item):
        # type: (dict) -> bool
        """Given a router item and remove the node_id

        The node_id will only be cleared if the ``connected_at`` matches up
        with the item's ``connected_at``.

        :returns: Whether the node was cleared or not.
        :raises:
            :exc:`ProvisionedThroughputExceededException` if dynamodb table
            exceeds throughput.

        """
        # Pop out the node_id
        node_id = item["node_id"]
        del item["node_id"]

        try:
            cond = "(node_id = :node) and (connected_at = :conn)"
            self.table.put_item(
                Item=item,
                ConditionExpression=cond,
                ExpressionAttributeValues={
                    ":node": node_id,
                    ":conn": item["connected_at"],
                },
            )
            return True
        except ClientError as ex:
            if (ex.response["Error"]["Code"] ==
                    "ProvisionedThroughputExceededException"):
                raise
            # UAID not found.
            return False


@attrs
class DatabaseManager(object):
    """Provides database access"""

    _router_conf = attrib()   # # type: DDBTableConfig
    _message_conf = attrib()  # # type: DDBTableConfig
    metrics = attrib()        # # type: IMetrics
    resource = attrib()       # type: DynamoDBResource

    router = attrib(default=None)                   # type: Optional[Router]
    message_tables = attrib(default=Factory(list))  # type: List[str]
    current_msg_month = attrib(init=False)          # type: Optional[str]
    current_month = attrib(init=False)              # type: Optional[int]
    _message = attrib(default=None)                 # type: Optional[Message]
    allow_table_rotation = attrib(default=True)     # type: Optional[bool]
    # for testing:

    def __attrs_post_init__(self):
        """Initialize sane defaults"""
        if self.allow_table_rotation:
            today = datetime.date.today()
            self.current_month = today.month
            self.current_msg_month = make_rotating_tablename(
                self._message_conf.tablename,
                date=today
            )
        else:
            # fetch out the last message table as the "current_msg_month"
            # Message may still init to this table if it recv's None, but
            # this makes the value explicit.
            resource = self.resource
            self.current_msg_month = resource.get_latest_message_tablename(
                prefix=self._message_conf.tablename
            )

        if not self.resource:
            self.resource = DynamoDBResource()

    @classmethod
    def from_config(cls,
                    conf,           # # type: AutopushConfig
                    resource=None,  # type: Optional[DynamoDBResource]
                    **kwargs        # type: Any
                    ):
        # type: (...) -> DatabaseManager
        """Create a DatabaseManager from the given config"""
        # metrics = autopush.metrics.from_config(conf)
        if not resource:
            resource = DynamoDBResource()
        return cls(
            router_conf=conf.router_table,
            message_conf=conf.message_table,
            # metrics=metrics,
            resource=resource,
            allow_table_rotation=conf.allow_table_rotation,
            **kwargs
        )

    def setup(self, preflight_uaid):
        # type: (str) -> None
        """Setup metrics, message tables and perform preflight_check"""
        self.metrics.start()
        self.setup_tables()
        preflight_check(self.message, self.router, preflight_uaid)

    def setup_tables(self):
        """Lookup or create the database tables"""
        self.router = Router(
            conf=self._router_conf,
            metrics=self.metrics,
            resource=self.resource)
        # Used to determine whether a connection is out of date with current
        # db objects. There are three noteworty cases:
        # 1 "Last Month" the table requires a rollover.
        # 2 "This Month" the most common case.
        # 3 "Next Month" where the system will soon be rolling over, but with
        #   timing, some nodes may roll over sooner. Ensuring the next month's
        #   table is present before the switchover is the main reason for this,
        #   just in case some nodes do switch sooner.
        self.create_initial_message_tables()
        self._message = Message(self.current_msg_month,
                                boto_resource=self.resource)

    @property
    def message(self):
        # type: () -> Message
        """Property that access the current message table"""
        if not self._message or isinstance(self._message, Attribute):
            self._message = self.message_table(self.current_msg_month)
        return self._message

    def message_table(self, tablename):
        return Message(tablename, boto_resource=self.resource)

    def _tomorrow(self):
        # type: () -> datetime.date
        return datetime.date.today() + datetime.timedelta(days=1)

    def create_initial_message_tables(self):
        """Initializes a dict of the initial rotating messages tables.

        An entry for last months table, an entry for this months table,
        an entry for tomorrow, if tomorrow is a new month.

        """
        if not self.allow_table_rotation:
            tablenames = self.resource.get_latest_message_tablenames(
                prefix=self._message_conf.tablename,
                previous=3
            )
            # Create the most recent table if it's not there.
            tablename = tablenames[-1]
            if not table_exists(tablename,
                                boto_resource=self.resource):
                create_message_table(
                    tablename=tablename,
                    read_throughput=self._message_conf.read_throughput,
                    write_throughput=self._message_conf.write_throughput,
                    boto_resource=self.resource
                )
            self.message_tables.extend(tablenames)
            return

        mconf = self._message_conf
        today = datetime.date.today()
        last_month = get_rotating_message_tablename(
            prefix=mconf.tablename,
            delta=-1,
            message_read_throughput=mconf.read_throughput,
            message_write_throughput=mconf.write_throughput,
            boto_resource=self.resource,
        )
        this_month = get_rotating_message_tablename(
            prefix=mconf.tablename,
            message_read_throughput=mconf.read_throughput,
            message_write_throughput=mconf.write_throughput,
            boto_resource=self.resource,
        )
        self.current_month = today.month
        self.current_msg_month = this_month
        self.message_tables = [last_month, this_month]
        if self._tomorrow().month != today.month:
            next_month = get_rotating_message_tablename(
                prefix=mconf.tablename,
                delta=1,
                message_read_throughput=mconf.read_throughput,
                message_write_throughput=mconf.write_throughput,
                boto_resource=self.resource,
            )
            self.message_tables.append(next_month)
#
#    @inlineCallbacks
#    def update_rotating_tables(self):
#        # type: () -> Generator
#        """This method is intended to be tasked to run periodically off the
#        twisted event hub to rotate tables.
#
#        When today is a new month from yesterday, then we swap out all the
#        table objects on the settings object.
#
#        """
#        if not self.allow_table_rotation:
#            returnValue(False)
#        mconf = self._message_conf
#        today = datetime.date.today()
#        tomorrow = self._tomorrow()
#        if ((tomorrow.month != today.month) and
#                sorted(self.message_tables)[-1] != tomorrow.month):
#            next_month = yield deferToThread(
#                get_rotating_message_tablename,
#                prefix=mconf.tablename,
#                delta=0,
#                date=tomorrow,
#                message_read_throughput=mconf.read_throughput,
#                message_write_throughput=mconf.write_throughput,
#                boto_resource=self.resource
#            )
#            self.message_tables.append(next_month)
#        if today.month == self.current_month:
#            # No change in month, we're fine.
#            returnValue(False)
#
#        # Get tables for the new month, and verify they exist before we
#        # try to switch over
#        message_table = yield deferToThread(
#            get_rotating_message_tablename,
#            prefix=mconf.tablename,
#            message_read_throughput=mconf.read_throughput,
#            message_write_throughput=mconf.write_throughput,
#            boto_resource=self.resource,
#        )
#
#        # Both tables found, safe to switch-over
#        self.current_month = today.month
#        self.current_msg_month = message_table
#        returnValue(True)
#