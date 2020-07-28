"""Rust Connection Node Integration Test

Differences from original integration test:

1. Connection node metrics can't be counted from the Python side.
2. Increment is only run after all messages are ack'd, rather than merely the
   last message as production currently uses.

"""
import logging
import os
import signal
import socket
import subprocess
import time
import uuid
from contextlib import contextmanager
from functools import wraps
from threading import Event, Thread
from unittest import SkipTest

import autopush.tests
import autopush.tests as ap_tests
import bottle
import ecdsa
import psutil
import requests
import twisted.internet.base
from autopush.config import AutopushConfig
from autopush.db import (Message, get_month, has_connected_this_month,
                         make_rotating_tablename)
from autopush.logging import begin_or_register
from autopush.main import (
    ConnectionApplication,
    EndpointApplication,
)
from autopush.tests.support import _TestingLogObserver
from autopush.tests.test_integration import (
    Client,
    _get_vapid,
)
from autopush.utils import base64url_encode
from cryptography.fernet import Fernet
from Queue import Empty, Queue
from twisted.internet.defer import inlineCallbacks, returnValue
from twisted.internet.threads import deferToThread
from twisted.logger import globalLogPublisher
from twisted.trial import unittest

log = logging.getLogger(__name__)

here_dir = os.path.abspath(os.path.dirname(__file__))
root_dir = os.path.dirname(here_dir)

twisted.internet.base.DelayedCall.debug = True

ROUTER_TABLE = os.environ.get("ROUTER_TABLE", "router_int_test")
MESSAGE_TABLE = os.environ.get("MESSAGE_TABLE", "message_int_test")
MSG_LIMIT = 20

CRYPTO_KEY = Fernet.generate_key()
CONNECTION_PORT = 9150
ENDPOINT_PORT = 9160
ROUTER_PORT = 9170
MP_CONNECTION_PORT = 9052
MP_ROUTER_PORT = 9072
RP_CONNECTION_PORT = 9054
RP_ROUTER_PORT = 9074

CN_SERVER = None
CN_MP_SERVER = None
MOCK_SERVER_THREAD = None
CN_QUEUES = []
STRICT_LOG_COUNTS = True


def get_free_port():
    s = socket.socket(socket.AF_INET, type=socket.SOCK_STREAM)
    s.bind(('localhost', 0))
    address, port = s.getsockname()
    s.close()
    return port


MOCK_SERVER_PORT = get_free_port()
MOCK_MP_SERVICES = {}
MOCK_MP_TOKEN = "Bearer {}".format(uuid.uuid4().hex)
MOCK_MP_POLLED = Event()
MOCK_SENTRY_QUEUE = Queue()


def enqueue_output(out, queue):
    for line in iter(out.readline, b''):
        queue.put(line)
    out.close()


def process_logs(testcase):
    """Process (print) the testcase logs (in tearDown).

    Ensures a maximum level of logs allowed to be emitted when running
    w/ a `--release` mode connection node

    """
    conn_count = sum(queue.qsize() for queue in CN_QUEUES)

    for queue in CN_QUEUES:
        is_empty = False
        while not is_empty:
            try:
                line = queue.get_nowait()
            except Empty:
                is_empty = True
            else:
                print(line)

    if not STRICT_LOG_COUNTS:
        return

    MSG = "endpoint node emitted excessive log statements, count: {} > max: {}"
    endpoint_count = len(testcase.logs)
    # Give an extra to endpoint for potential startup log messages
    # (e.g. when running tests individually)
    max_endpoint_logs = testcase.max_endpoint_logs + 1
    assert endpoint_count <= max_endpoint_logs, MSG.format(
        endpoint_count, max_endpoint_logs)

    MSG = "conn node emitted excessive log statements, count: {} > max: {}"
    assert conn_count <= testcase.max_conn_logs, MSG.format(
        conn_count, testcase.max_conn_logs)


def max_logs(endpoint=None, conn=None):
    """Adjust max_endpoint/conn_logs values for individual test cases.

    They're utilized by the process_logs function

    """
    def max_logs_decorator(func):
        @wraps(func)
        def wrapper(self, *args, **kwargs):
            if endpoint is not None:
                self.max_endpoint_logs = endpoint
            if conn is not None:
                self.max_conn_logs = conn
            return func(self, *args, **kwargs)
        return wrapper
    return max_logs_decorator


@bottle.get("/v1/broadcasts")
def broadcast_handler():
    assert bottle.request.headers["Authorization"] == MOCK_MP_TOKEN
    MOCK_MP_POLLED.set()
    return dict(broadcasts=MOCK_MP_SERVICES)


@bottle.post("/api/1/store/")
def sentry_handler():
    content = bottle.request.json
    MOCK_SENTRY_QUEUE.put(content)
    return {
        "id": "fc6d8c0c43fc4630ad850ee518f1b9d0"
    }


class CustomClient(Client):
    def send_bad_data(self):
        self.ws.send("bad-data")


def setup_module():
    global CN_SERVER, CN_QUEUES, CN_MP_SERVER, MOCK_SERVER_THREAD, \
        STRICT_LOG_COUNTS
    ap_tests.ddb_jar = os.path.join(root_dir, "ddb", "DynamoDBLocal.jar")
    ap_tests.ddb_lib_dir = os.path.join(root_dir, "ddb", "DynamoDBLocal_lib")
    ap_tests.setUp()
    logging.getLogger('boto').setLevel(logging.CRITICAL)
    if "SKIP_INTEGRATION" in os.environ:  # pragma: nocover
        raise SkipTest("Skipping integration tests")

    conn_conf = dict(
        hostname='localhost',
        port=CONNECTION_PORT,
        endpoint_hostname="localhost",
        endpoint_port=ENDPOINT_PORT,
        router_port=ROUTER_PORT,
        endpoint_scheme='http',
        statsd_host="",
        router_tablename=ROUTER_TABLE,
        message_tablename=MESSAGE_TABLE,
        crypto_key="[{}]".format(CRYPTO_KEY),
        auto_ping_interval=60.0,
        auto_ping_timeout=10.0,
        close_handshake_timeout=5,
        max_connections=5000,
        human_logs="true",
        msg_limit=MSG_LIMIT,
    )
    rust_bin = root_dir + "/target/release/autopush_rs"
    possible_paths = ["/target/debug/autopush_rs",
                      "/autopush_rs/target/release/autopush_rs",
                      "/autopush_rs/target/debug/autopush_rs"]
    while possible_paths and not os.path.exists(rust_bin):  # pragma: nocover
        rust_bin = root_dir + possible_paths.pop(0)

    if 'release' not in rust_bin:
        # disable checks for chatty debug mode autopush-rs
        STRICT_LOG_COUNTS = False

    # Setup the environment
    for key, val in conn_conf.items():
        key = "autopush_" + key
        os.environ[key.upper()] = str(val)
    # Sentry API mock
    os.environ["SENTRY_DSN"] = 'http://foo:bar@localhost:{}/1'.format(
        MOCK_SERVER_PORT
    )

    cmd = [rust_bin]
    CN_SERVER = subprocess.Popen(cmd, shell=True, env=os.environ,
                                 stdout=subprocess.PIPE,
                                 stderr=subprocess.PIPE,
                                 universal_newlines=True)
    # Spin up the readers to dump the output from stdout/stderr
    out_q = Queue()
    t = Thread(target=enqueue_output, args=(CN_SERVER.stdout, out_q))
    t.daemon = True  # thread dies with the program
    t.start()
    err_q = Queue()
    t = Thread(target=enqueue_output, args=(CN_SERVER.stderr, err_q))
    t.daemon = True  # thread dies with the program
    t.start()
    CN_QUEUES.extend([out_q, err_q])

    MOCK_SERVER_THREAD = Thread(
        target=bottle.run,
        kwargs=dict(
            port=MOCK_SERVER_PORT, debug=True
        ))
    MOCK_SERVER_THREAD.setDaemon(True)
    MOCK_SERVER_THREAD.start()

    # Setup the megaphone connection node
    megaphone_api_url = 'http://localhost:{port}/v1/broadcasts'.format(
        port=MOCK_SERVER_PORT)
    conn_conf.update(dict(
        port=MP_CONNECTION_PORT,
        endpoint_port=ENDPOINT_PORT,
        router_port=MP_ROUTER_PORT,
        auto_ping_interval=0.5,
        auto_ping_timeout=10.0,
        close_handshake_timeout=5,
        max_connections=5000,
        megaphone_api_url=megaphone_api_url,
        megaphone_api_token=MOCK_MP_TOKEN,
        megaphone_poll_interval=1,
    ))

    # Setup the environment
    for key, val in conn_conf.items():
        key = "autopush_" + key
        os.environ[key.upper()] = str(val)

    cmd = [rust_bin]
    CN_MP_SERVER = subprocess.Popen(cmd, shell=True, env=os.environ)
    time.sleep(2)


def teardown_module():
    global CN_SERVER, CN_MP_SERVER
    ap_tests.tearDown()
    # This kinda sucks, but its the only way to nuke the child procs
    proc = psutil.Process(pid=CN_SERVER.pid)
    child_procs = proc.children(recursive=True)
    for p in [proc] + child_procs:
        os.kill(p.pid, signal.SIGTERM)
        CN_SERVER.wait()

    proc = psutil.Process(pid=CN_MP_SERVER.pid)
    child_procs = proc.children(recursive=True)
    for p in [proc] + child_procs:
        os.kill(p.pid, signal.SIGTERM)
        CN_MP_SERVER.wait()


class TestRustWebPush(unittest.TestCase):
    _endpoint_defaults = dict(
        hostname='localhost',
        port=ENDPOINT_PORT,
        endpoint_port=ENDPOINT_PORT,
        endpoint_scheme='http',
        router_port=ROUTER_PORT,
        statsd_host=None,
        router_table=dict(tablename=ROUTER_TABLE),
        message_table=dict(tablename=MESSAGE_TABLE),
        use_cryptography=True,
    )

    # Max log lines allowed to be emitted by each node type
    max_endpoint_logs = 8
    max_conn_logs = 3

    def start_ep(self, ep_conf):
        # Endpoint HTTP router
        self.ep = ep = EndpointApplication(
            ep_conf,
            resource=autopush.tests.boto_resource
        )
        ep.setup(rotate_tables=False)
        ep.startService()
        self.addCleanup(ep.stopService)

    def setUp(self):
        self.logs = _TestingLogObserver()
        begin_or_register(self.logs)
        self.addCleanup(globalLogPublisher.removeObserver, self.logs)

        self._ep_conf = AutopushConfig(
            crypto_key=CRYPTO_KEY,
            **self.endpoint_kwargs()
        )
        self.start_ep(self._ep_conf)

    def tearDown(self):
        process_logs(self)
        while not MOCK_SENTRY_QUEUE.empty():
            MOCK_SENTRY_QUEUE.get_nowait()

    def endpoint_kwargs(self):
        return self._endpoint_defaults

    @inlineCallbacks
    def quick_register(self, sslcontext=None):
        client = Client("ws://localhost:{}/".format(CONNECTION_PORT),
                        sslcontext=sslcontext)
        yield client.connect()
        yield client.hello()
        yield client.register()
        returnValue(client)

    @inlineCallbacks
    def shut_down(self, client=None):
        if client:
            yield client.disconnect()

    @contextmanager
    def legacy_endpoint(self):
        self.ep.conf._notification_legacy = True
        yield
        self.ep.conf._notification_legacy = False

    @property
    def _ws_url(self):
        return "ws://localhost:{}/".format(CONNECTION_PORT)

    @inlineCallbacks
    @max_logs(conn=4)
    def test_sentry_output(self):
        # Ensure bad data doesn't throw errors
        client = CustomClient(self._ws_url)
        yield client.connect()
        yield client.hello()
        yield client.send_bad_data()
        yield self.shut_down(client)

        # LogCheck does throw an error every time
        requests.get("http://localhost:{}/v1/err/crit".format(CONNECTION_PORT))
        data = MOCK_SENTRY_QUEUE.get(timeout=1)
        assert data["exception"]["values"][0]["value"] == "LogCheck"

    @inlineCallbacks
    def test_hello_echo(self):
        client = Client(self._ws_url)
        yield client.connect()
        result = yield client.hello()
        assert result != {}
        assert result["use_webpush"] is True
        yield self.shut_down(client)

    @inlineCallbacks
    def test_hello_with_bad_prior_uaid(self):
        non_uaid = uuid.uuid4().hex
        client = Client(self._ws_url)
        yield client.connect()
        result = yield client.hello(uaid=non_uaid)
        assert result != {}
        assert result["uaid"] != non_uaid
        assert result["use_webpush"] is True
        yield self.shut_down(client)

    @inlineCallbacks
    def test_basic_delivery(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        result = yield client.send_notification(data=data)
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        yield self.shut_down(client)

    @inlineCallbacks
    def test_topic_basic_delivery(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        result = yield client.send_notification(data=data, topic="Inbox")
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        yield self.shut_down(client)

    @inlineCallbacks
    def test_topic_replacement_delivery(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        yield client.send_notification(data=data, topic="Inbox")
        yield client.send_notification(data=data2, topic="Inbox")
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data2)
        assert result["messageType"] == "notification"
        result = yield client.get_notification()
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(conn=4)
    def test_topic_no_delivery_on_reconnect(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        yield client.send_notification(data=data, topic="Inbox")
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=10)
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        yield client.ack(result["channelID"], result["version"])
        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result is None
        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        yield self.shut_down(client)

    @inlineCallbacks
    def test_basic_delivery_with_vapid(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        vapid_info = _get_vapid()
        result = yield client.send_notification(data=data, vapid=vapid_info)
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        assert self.logs.logged_ci(lambda ci: 'router_key' in ci)
        yield self.shut_down(client)

    @inlineCallbacks
    def test_basic_delivery_with_invalid_vapid(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        vapid_info = _get_vapid()
        vapid_info['crypto-key'] = "invalid"
        yield client.send_notification(
            data=data,
            vapid=vapid_info,
            status=401)
        yield self.shut_down(client)

    @inlineCallbacks
    def test_basic_delivery_with_invalid_vapid_exp(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        vapid_info = _get_vapid(
            payload={"aud": "https://pusher_origin.example.com",
                     "exp": '@',
                     "sub": "mailto:admin@example.com"})
        vapid_info['crypto-key'] = "invalid"
        yield client.send_notification(
            data=data,
            vapid=vapid_info,
            status=401)
        yield self.shut_down(client)

    @inlineCallbacks
    def test_basic_delivery_with_invalid_vapid_auth(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        vapid_info = _get_vapid()
        vapid_info['auth'] = ""
        yield client.send_notification(
            data=data,
            vapid=vapid_info,
            status=401)
        yield self.shut_down(client)

    @inlineCallbacks
    def test_basic_delivery_with_invalid_signature(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        vapid_info = _get_vapid(
            payload={"aud": "https://pusher_origin.example.com",
                     "sub": "mailto:admin@example.com"})
        vapid_info['auth'] = vapid_info['auth'][:-3] + "bad"
        yield client.send_notification(
            data=data,
            vapid=vapid_info,
            status=401)
        yield self.shut_down(client)

    @inlineCallbacks
    def test_basic_delivery_with_invalid_vapid_ckey(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        vapid_info = _get_vapid()
        vapid_info['crypto-key'] = "invalid|"
        yield client.send_notification(
            data=data,
            vapid=vapid_info,
            status=401)
        yield self.shut_down(client)

    @inlineCallbacks
    def test_delivery_repeat_without_ack(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] == base64url_encode(data)

        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] == base64url_encode(data)
        yield self.shut_down(client)

    @inlineCallbacks
    def test_repeat_delivery_with_disconnect_without_ack(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        result = yield client.send_notification(data=data)
        assert result != {}
        assert result["data"] == base64url_encode(data)
        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] == base64url_encode(data)
        yield self.shut_down(client)

    @inlineCallbacks
    def test_multiple_delivery_repeat_without_ack(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data)
        yield client.send_notification(data=data2)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])

        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        yield self.shut_down(client)

    @inlineCallbacks
    def test_multiple_legacy_delivery_with_single_ack(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        with self.legacy_endpoint():
            yield client.send_notification(data=data)
            yield client.send_notification(data=data2)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        yield client.ack(result["channelID"], result["version"])

        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        assert result["messageType"] == "notification"
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    def test_topic_expired(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data, ttl=1, topic="test")
        yield client.sleep(2)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        result = yield client.send_notification(data=data, topic="test")
        assert result != {}
        assert result["data"] == base64url_encode(data)
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(conn=4)
    def test_multiple_delivery_with_single_ack(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data)
        yield client.send_notification(data=data2)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] == base64url_encode(data)
        result2 = yield client.get_notification(timeout=0.5)
        assert result2 != {}
        assert result2["data"] == base64url_encode(data2)
        yield client.ack(result["channelID"], result["version"])

        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        result2 = yield client.get_notification()
        assert result2 != {}
        assert result2["data"] == base64url_encode(data2)
        yield client.ack(result["channelID"], result["version"])
        yield client.ack(result2["channelID"], result2["version"])

        # Verify no messages are delivered
        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    def test_multiple_delivery_with_multiple_ack(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data)
        yield client.send_notification(data=data2)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        result2 = yield client.get_notification()
        assert result2 != {}
        assert result2["data"] in map(base64url_encode, [data, data2])
        yield client.ack(result2["channelID"], result2["version"])
        yield client.ack(result["channelID"], result["version"])

        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    def test_no_delivery_to_unregistered(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()  # type: Client
        assert client.channels
        chan = client.channels.keys()[0]

        result = yield client.send_notification(data=data)
        assert result["channelID"] == chan
        assert result["data"] == base64url_encode(data)
        yield client.ack(result["channelID"], result["version"])

        yield client.unregister(chan)
        result = yield client.send_notification(data=data, status=410)

        # Verify cache-control
        assert client.notif_response.getheader("Cache-Control") == \
            "max-age=86400"

        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    def test_ttl_0_connected(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        result = yield client.send_notification(data=data, ttl=0)
        assert result is not None
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        yield self.shut_down(client)

    @inlineCallbacks
    def test_ttl_0_not_connected(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        yield client.send_notification(data=data, ttl=0)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    def test_ttl_expired(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        yield client.send_notification(data=data, ttl=1)
        time.sleep(1)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(endpoint=28)
    def test_ttl_batch_expired_and_good_one(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        for x in range(0, 12):
            yield client.send_notification(data=data, ttl=1)

        yield client.send_notification(data=data2)
        time.sleep(1)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=4)
        assert result is not None
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data2)
        assert result["messageType"] == "notification"
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(endpoint=28)
    def test_ttl_batch_partly_expired_and_good_one(self):
        data = str(uuid.uuid4())
        data1 = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        for x in range(0, 6):
            yield client.send_notification(data=data)

        for x in range(0, 6):
            yield client.send_notification(data=data1, ttl=1)

        yield client.send_notification(data=data2)
        time.sleep(1)
        yield client.connect()
        yield client.hello()

        # Pull out and ack the first
        for x in range(0, 6):
            result = yield client.get_notification(timeout=4)
            assert result is not None
            assert result["data"] == base64url_encode(data)
            yield client.ack(result["channelID"], result["version"])

        # Should have one more that is data2, this will only arrive if the
        # other six were acked as that hits the batch size
        result = yield client.get_notification(timeout=4)
        assert result is not None
        assert result["data"] == base64url_encode(data2)

        # No more
        result = yield client.get_notification()
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    def test_message_without_crypto_headers(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        result = yield client.send_notification(data=data, use_header=False,
                                                status=400)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    def test_empty_message_without_crypto_headers(self):
        client = yield self.quick_register()
        result = yield client.send_notification(use_header=False)
        assert result is not None
        assert result["messageType"] == "notification"
        assert "headers" not in result
        assert "data" not in result
        yield client.ack(result["channelID"], result["version"])

        yield client.disconnect()
        yield client.send_notification(use_header=False)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result is not None
        assert "headers" not in result
        assert "data" not in result
        yield client.ack(result["channelID"], result["version"])

        yield self.shut_down(client)

    @inlineCallbacks
    def test_empty_message_with_crypto_headers(self):
        client = yield self.quick_register()
        result = yield client.send_notification()
        assert result is not None
        assert result["messageType"] == "notification"
        assert "headers" not in result
        assert "data" not in result

        result2 = yield client.send_notification()
        # We shouldn't store headers for blank messages.
        assert result2 is not None
        assert result2["messageType"] == "notification"
        assert "headers" not in result2
        assert "data" not in result2

        yield client.ack(result["channelID"], result["version"])
        yield client.ack(result2["channelID"], result2["version"])

        yield client.disconnect()
        yield client.send_notification()
        yield client.connect()
        yield client.hello()
        result3 = yield client.get_notification()
        assert result3 is not None
        assert "headers" not in result3
        assert "data" not in result3
        yield client.ack(result3["channelID"], result3["version"])

        yield self.shut_down(client)

    @inlineCallbacks
    def test_delete_saved_notification(self):
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        chan = client.channels.keys()[0]
        yield client.send_notification()
        yield client.delete_notification(chan)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification()
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    def test_with_key(self):
        private_key = ecdsa.SigningKey.generate(curve=ecdsa.NIST256p)
        claims = {"aud": "http://example.com",
                  "exp": int(time.time()) + 86400,
                  "sub": "a@example.com"}
        vapid = _get_vapid(private_key, claims)
        pk_hex = vapid['crypto-key']
        chid = str(uuid.uuid4())
        client = Client("ws://localhost:{}/".format(CONNECTION_PORT))
        yield client.connect()
        yield client.hello()
        yield client.register(chid=chid, key=pk_hex)

        # Send an update with a properly formatted key.
        yield client.send_notification(vapid=vapid)

        # now try an invalid key.
        new_key = ecdsa.SigningKey.generate(curve=ecdsa.NIST256p)
        vapid = _get_vapid(new_key, claims)

        yield client.send_notification(
            vapid=vapid,
            status=401)

        yield self.shut_down(client)

    @inlineCallbacks
    def test_with_bad_key(self):
        chid = str(uuid.uuid4())
        client = Client("ws://localhost:{}/".format(CONNECTION_PORT))
        yield client.connect()
        yield client.hello()
        result = yield client.register(chid=chid, key="af1883%&!@#*(",
                                       status=400)
        assert result["status"] == 400

        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(endpoint=44)
    def test_msg_limit(self):
        client = yield self.quick_register()
        uaid = client.uaid
        yield client.disconnect()
        for i in range(MSG_LIMIT + 1):
            yield client.send_notification()
        yield client.connect()
        yield client.hello()
        assert client.uaid == uaid
        for i in range(MSG_LIMIT):
            result = yield client.get_notification()
            assert result is not None
            yield client.ack(result["channelID"], result["version"])
        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        assert client.uaid != uaid
        yield self.shut_down(client)

    @inlineCallbacks
    def test_can_ping(self):
        client = yield self.quick_register()
        yield client.ping()
        assert client.ws.connected
        try:
            yield client.ping()
        except AssertionError:
            # pinging too quickly should disconnect without a valid ping
            # repsonse
            pass
        assert not client.ws.connected
        yield self.shut_down(client)


class TestRustWebPushBroadcast(unittest.TestCase):
    _endpoint_defaults = dict(
        hostname='localhost',
        port=ENDPOINT_PORT,
        endpoint_port=ENDPOINT_PORT,
        endpoint_scheme='http',
        router_port=MP_ROUTER_PORT,
        statsd_host=None,
        router_table=dict(tablename=ROUTER_TABLE),
        message_table=dict(tablename=MESSAGE_TABLE),
        use_cryptography=True,
    )

    _conn_defaults = dict(
        hostname='localhost',
        port=RP_CONNECTION_PORT,
        endpoint_port=ENDPOINT_PORT,
        router_port=RP_ROUTER_PORT,
        endpoint_scheme='http',
        statsd_host=None,
        router_table=dict(tablename=ROUTER_TABLE),
        message_table=dict(tablename=MESSAGE_TABLE),
        use_cryptography=True,
        human_logs=False,
    )

    max_endpoint_logs = 4
    max_conn_logs = 1

    def start_ep(self, ep_conf):
        # Endpoint HTTP router
        self.ep = ep = EndpointApplication(
            ep_conf,
            resource=autopush.tests.boto_resource
        )
        ep.setup(rotate_tables=False)
        ep.startService()
        self.addCleanup(ep.stopService)

    def setUp(self):
        self.logs = _TestingLogObserver()
        begin_or_register(self.logs)
        self.addCleanup(globalLogPublisher.removeObserver, self.logs)

        self._ep_conf = AutopushConfig(
            crypto_key=CRYPTO_KEY,
            **self.endpoint_kwargs()
        )

        self.start_ep(self._ep_conf)

        # Create a Python connection application for accessing the db
        self._conn_conf = AutopushConfig(
            crypto_key=CRYPTO_KEY,
            **self.conn_kwargs()
        )
        self.conn = conn = ConnectionApplication(
            self._conn_conf,
            resource=autopush.tests.boto_resource,
        )
        conn.setup(rotate_tables=True)

    def tearDown(self):
        process_logs(self)

    def endpoint_kwargs(self):
        return self._endpoint_defaults

    def conn_kwargs(self):
        return self._conn_defaults

    @inlineCallbacks
    def quick_register(self, sslcontext=None, connection_port=None):
        conn_port = connection_port or MP_CONNECTION_PORT
        client = Client("ws://localhost:{}/".format(conn_port),
                        sslcontext=sslcontext)
        yield client.connect()
        yield client.hello()
        yield client.register()
        returnValue(client)

    @inlineCallbacks
    def shut_down(self, client=None):
        if client:
            yield client.disconnect()

    @contextmanager
    def legacy_endpoint(self):
        self.ep.conf._notification_legacy = True
        yield
        self.ep.conf._notification_legacy = False

    @property
    def _ws_url(self):
        return "ws://localhost:{}/".format(MP_CONNECTION_PORT)

    @inlineCallbacks
    def test_broadcast_update_on_connect(self):
        global MOCK_MP_SERVICES
        MOCK_MP_SERVICES = {"kinto:123": "ver1"}
        MOCK_MP_POLLED.clear()
        MOCK_MP_POLLED.wait(timeout=5)

        old_ver = {"kinto:123": "ver0"}
        client = Client(self._ws_url)
        yield client.connect()
        result = yield client.hello(services=old_ver)
        assert result != {}
        assert result["use_webpush"] is True
        assert result["broadcasts"]["kinto:123"] == "ver1"

        MOCK_MP_SERVICES = {"kinto:123": "ver2"}
        MOCK_MP_POLLED.clear()
        MOCK_MP_POLLED.wait(timeout=5)

        result = yield client.get_broadcast(2)
        assert result["broadcasts"]["kinto:123"] == "ver2"

        yield self.shut_down(client)

    @inlineCallbacks
    def test_broadcast_update_on_connect_with_errors(self):
        global MOCK_MP_SERVICES
        MOCK_MP_SERVICES = {"kinto:123": "ver1"}
        MOCK_MP_POLLED.clear()
        MOCK_MP_POLLED.wait(timeout=5)

        old_ver = {"kinto:123": "ver0", "kinto:456": "ver1"}
        client = Client(self._ws_url)
        yield client.connect()
        result = yield client.hello(services=old_ver)
        assert result != {}
        assert result["use_webpush"] is True
        assert result["broadcasts"]["kinto:123"] == "ver1"
        assert result["broadcasts"]["errors"][
                   "kinto:456"] == "Broadcast not found"
        yield self.shut_down(client)

    @inlineCallbacks
    def test_broadcast_subscribe(self):
        global MOCK_MP_SERVICES
        MOCK_MP_SERVICES = {"kinto:123": "ver1"}
        MOCK_MP_POLLED.clear()
        MOCK_MP_POLLED.wait(timeout=5)

        old_ver = {"kinto:123": "ver0"}
        client = Client(self._ws_url)
        yield client.connect()
        result = yield client.hello()
        assert result != {}
        assert result["use_webpush"] is True
        assert result["broadcasts"] == {}

        client.broadcast_subscribe(old_ver)
        result = yield client.get_broadcast()
        assert result["broadcasts"]["kinto:123"] == "ver1"

        MOCK_MP_SERVICES = {"kinto:123": "ver2"}
        MOCK_MP_POLLED.clear()
        MOCK_MP_POLLED.wait(timeout=5)

        result = yield client.get_broadcast(2)
        assert result["broadcasts"]["kinto:123"] == "ver2"

        yield self.shut_down(client)

    @inlineCallbacks
    def test_broadcast_subscribe_with_errors(self):
        global MOCK_MP_SERVICES
        MOCK_MP_SERVICES = {"kinto:123": "ver1"}
        MOCK_MP_POLLED.clear()
        MOCK_MP_POLLED.wait(timeout=5)

        old_ver = {"kinto:123": "ver0", "kinto:456": "ver1"}
        client = Client(self._ws_url)
        yield client.connect()
        result = yield client.hello()
        assert result != {}
        assert result["use_webpush"] is True
        assert result["broadcasts"] == {}

        client.broadcast_subscribe(old_ver)
        result = yield client.get_broadcast()
        assert result["broadcasts"]["kinto:123"] == "ver1"
        assert result["broadcasts"]["errors"][
                   "kinto:456"] == "Broadcast not found"

        yield self.shut_down(client)

    @inlineCallbacks
    def test_broadcast_no_changes(self):
        global MOCK_MP_SERVICES
        MOCK_MP_SERVICES = {"kinto:123": "ver1"}
        MOCK_MP_POLLED.clear()
        MOCK_MP_POLLED.wait(timeout=5)

        old_ver = {"kinto:123": "ver1"}
        client = Client(self._ws_url)
        yield client.connect()
        result = yield client.hello(services=old_ver)
        assert result != {}
        assert result["use_webpush"] is True
        assert result["broadcasts"] == {}

        yield self.shut_down(client)

    @inlineCallbacks
    def test_webpush_monthly_rotation(self):
        client = yield self.quick_register()
        yield client.disconnect()

        # Move the client back one month to the past
        last_month = make_rotating_tablename(
            prefix=self.conn.conf.message_table.tablename, delta=-1)
        lm_message = Message(last_month, boto_resource=self.conn.db.resource)
        yield deferToThread(
            self.conn.db.router.update_message_month,
            client.uaid,
            last_month,
        )

        # Verify the move
        c = yield deferToThread(self.conn.db.router.get_uaid,
                                client.uaid)
        assert c["current_month"] == last_month

        # Verify last_connect is current, then move that back
        assert has_connected_this_month(c)
        today = get_month(delta=-1)
        last_connect = int("%s%s020001" % (today.year,
                                           str(today.month).zfill(2)))

        yield deferToThread(
            self.conn.db.router._update_last_connect,
            client.uaid,
            last_connect)
        c = yield deferToThread(self.conn.db.router.get_uaid,
                                client.uaid)
        assert has_connected_this_month(c) is False

        # Move the clients channels back one month
        exists, chans = yield deferToThread(
            self.conn.db.message.all_channels,
            client.uaid
        )
        assert exists is True
        assert len(chans) == 1
        yield deferToThread(
            lm_message.save_channels,
            client.uaid,
            chans,
        )

        # Remove the channels entry entirely from this month
        yield deferToThread(
            self.conn.db.message.table.delete_item,
            Key={'uaid': client.uaid, 'chidmessageid': ' '}
        )

        # Verify the channel is gone
        exists, chans = yield deferToThread(
            self.conn.db.message.all_channels,
            client.uaid,
        )
        assert exists is False
        assert len(chans) == 0

        # Send in a notification, verify it landed in last months notification
        # table
        data = uuid.uuid4().hex
        yield client.send_notification(data=data)
        ts, notifs = yield deferToThread(lm_message.fetch_timestamp_messages,
                                         uuid.UUID(client.uaid),
                                         " ")
        assert len(notifs) == 1

        # Connect the client, verify the migration
        yield client.connect()
        yield client.hello()

        # Pull down the notification
        result = yield client.get_notification()
        chan = client.channels.keys()[0]
        assert result is not None
        assert chan == result["channelID"]

        # Acknowledge the notification, which triggers the migration
        yield client.ack(chan, result["version"])

        # Wait up to 4 seconds for the table rotation to occur
        start = time.time()
        while time.time()-start < 4:
            c = yield deferToThread(
                self.conn.db.router.get_uaid,
                client.uaid)
            if c["current_month"] == self.conn.db.current_msg_month:
                break
            else:
                yield deferToThread(time.sleep, 0.2)

        # Verify the month update in the router table
        c = yield deferToThread(
            self.conn.db.router.get_uaid,
            client.uaid)
        assert c["current_month"] == self.conn.db.current_msg_month

        # Verify the client moved last_connect
        assert has_connected_this_month(c) is True

        # Verify the channels were moved
        exists, chans = yield deferToThread(
            self.conn.db.message.all_channels,
            client.uaid
        )
        assert exists is True
        assert len(chans) == 1
        yield self.shut_down(client)

    @inlineCallbacks
    def test_webpush_monthly_rotation_prior_record_exists(self):
        client = yield self.quick_register()
        yield client.disconnect()

        # Move the client back one month to the past
        last_month = make_rotating_tablename(
            prefix=self.conn.conf.message_table.tablename, delta=-1)
        lm_message = Message(last_month,
                             boto_resource=autopush.tests.boto_resource)
        yield deferToThread(
            self.conn.db.router.update_message_month,
            client.uaid,
            last_month,
        )

        # Verify the move
        c = yield deferToThread(self.conn.db.router.get_uaid,
                                client.uaid)
        assert c["current_month"] == last_month

        # Verify last_connect is current, then move that back
        assert has_connected_this_month(c)
        today = get_month(delta=-1)
        yield deferToThread(
            self.conn.db.router._update_last_connect,
            client.uaid,
            int("%s%s020001" % (today.year, str(today.month).zfill(2))),
        )
        c = yield deferToThread(self.conn.db.router.get_uaid, client.uaid)
        assert has_connected_this_month(c) is False

        # Move the clients channels back one month
        exists, chans = yield deferToThread(
            self.conn.db.message.all_channels,
            client.uaid,
        )
        assert exists is True
        assert len(chans) == 1
        yield deferToThread(
            lm_message.save_channels,
            client.uaid,
            chans,
        )

        # Send in a notification, verify it landed in last months notification
        # table
        data = uuid.uuid4().hex
        yield client.send_notification(data=data)
        _, notifs = yield deferToThread(lm_message.fetch_timestamp_messages,
                                        uuid.UUID(client.uaid),
                                        " ")
        assert len(notifs) == 1

        # Connect the client, verify the migration
        yield client.connect()
        yield client.hello()

        # Pull down the notification
        result = yield client.get_notification()
        chan = client.channels.keys()[0]
        assert result is not None
        assert chan == result["channelID"]

        # Acknowledge the notification, which triggers the migration
        yield client.ack(chan, result["version"])

        # Wait up to 4 seconds for the table rotation to occur
        start = time.time()
        while time.time()-start < 4:
            c = yield deferToThread(
                self.conn.db.router.get_uaid,
                client.uaid)
            if c["current_month"] == self.conn.db.current_msg_month:
                break
            else:
                yield deferToThread(time.sleep, 0.2)

        # Verify the month update in the router table
        c = yield deferToThread(self.conn.db.router.get_uaid, client.uaid)
        assert c["current_month"] == self.conn.db.current_msg_month

        # Verify the client moved last_connect
        assert has_connected_this_month(c) is True

        # Verify the channels were moved
        exists, chans = yield deferToThread(
            self.conn.db.message.all_channels,
            client.uaid
        )
        assert exists is True
        assert len(chans) == 1
        yield self.shut_down(client)

    @inlineCallbacks
    def test_webpush_monthly_rotation_no_channels(self):
        client = Client("ws://localhost:{}/".format(MP_CONNECTION_PORT))
        yield client.connect()
        yield client.hello()
        yield client.disconnect()

        # Move the client back one month to the past
        last_month = make_rotating_tablename(
            prefix=self.conn.conf.message_table.tablename, delta=-1)
        yield deferToThread(
            self.conn.db.router.update_message_month,
            client.uaid,
            last_month
        )

        # Verify the move
        c = yield deferToThread(self.conn.db.router.get_uaid,
                                client.uaid
                                )
        assert c["current_month"] == last_month

        # Verify there's no channels
        exists, chans = yield deferToThread(
            self.conn.db.message.all_channels,
            client.uaid,
        )
        assert exists is False
        assert len(chans) == 0

        # Connect the client, verify the migration
        yield client.connect()
        yield client.hello()

        # Wait up to 2 seconds for the table rotation to occur
        start = time.time()
        while time.time()-start < 2:
            c = yield deferToThread(
                self.conn.db.router.get_uaid,
                client.uaid,
            )
            if c["current_month"] == self.conn.db.current_msg_month:
                break
            else:
                yield deferToThread(time.sleep, 0.2)

        # Verify the month update in the router table
        c = yield deferToThread(self.conn.db.router.get_uaid,
                                client.uaid)
        assert c["current_month"] == self.conn.db.current_msg_month
        yield self.shut_down(client)

    @inlineCallbacks
    def test_webpush_monthly_rotation_old_user_dropped(self):
        client = yield self.quick_register()
        uaid = client.uaid
        yield client.disconnect()

        # Move the client back 2 months to the past
        old_month = make_rotating_tablename(
            prefix=self.conn.conf.message_table.tablename, delta=-3)
        yield deferToThread(
            self.conn.db.router.update_message_month,
            client.uaid,
            old_month
        )
        _ = Message(old_month, boto_resource=autopush.tests.boto_resource)

        # Verify the move
        c = yield deferToThread(self.conn.db.router.get_uaid, client.uaid)
        assert c["current_month"] == old_month

        # Connect the client and verify its uaid was dropped
        yield client.connect()
        yield client.hello()
        assert client.uaid != uaid


class TestRustAndPythonWebPush(unittest.TestCase):
    _endpoint_defaults = dict(
        hostname='localhost',
        port=ENDPOINT_PORT,
        endpoint_port=ENDPOINT_PORT,
        endpoint_scheme='http',
        router_port=RP_ROUTER_PORT,
        statsd_host=None,
        router_table=dict(tablename=ROUTER_TABLE),
        message_table=dict(tablename=MESSAGE_TABLE),
        use_cryptography=True,
    )

    _conn_defaults = dict(
        hostname='localhost',
        port=RP_CONNECTION_PORT,
        endpoint_port=ENDPOINT_PORT,
        router_port=RP_ROUTER_PORT,
        endpoint_scheme='http',
        statsd_host=None,
        router_table=dict(tablename=ROUTER_TABLE),
        message_table=dict(tablename=MESSAGE_TABLE),
        use_cryptography=True,
        human_logs=False,
    )

    max_endpoint_logs = 1
    max_conn_logs = 3

    def start_ep(self, ep_conf):
        # Endpoint HTTP router
        self.ep = ep = EndpointApplication(
            ep_conf,
            resource=autopush.tests.boto_resource
        )
        ep.setup(rotate_tables=False)
        ep.startService()
        self.addCleanup(ep.stopService)

    def start_conn(self, conn_conf):
        # Startup only the Python connection application as we will use
        # the module global Rust one as well
        self.conn = conn = ConnectionApplication(
            conn_conf,
            resource=autopush.tests.boto_resource,
        )
        conn.setup(rotate_tables=False)
        conn.startService()
        self.addCleanup(conn.stopService)

    def setUp(self):
        self.logs = _TestingLogObserver()
        begin_or_register(self.logs)
        self.addCleanup(globalLogPublisher.removeObserver, self.logs)

        self._ep_conf = AutopushConfig(
            crypto_key=CRYPTO_KEY,
            **self.endpoint_kwargs()
        )
        self._conn_conf = AutopushConfig(
            crypto_key=CRYPTO_KEY,
            **self.conn_kwargs()
        )

        self.start_ep(self._ep_conf)
        self.start_conn(self._conn_conf)

    def tearDown(self):
        process_logs(self)

    def endpoint_kwargs(self):
        return self._endpoint_defaults

    def conn_kwargs(self):
        return self._conn_defaults

    @inlineCallbacks
    def quick_register(self, sslcontext=None, connection_port=None):
        conn_port = connection_port or RP_CONNECTION_PORT
        client = Client("ws://localhost:{}/".format(conn_port),
                        sslcontext=sslcontext)
        yield client.connect()
        yield client.hello()
        yield client.register()
        returnValue(client)

    @inlineCallbacks
    def shut_down(self, client=None):
        if client:
            yield client.disconnect()

    @inlineCallbacks
    @max_logs(endpoint=41)
    def test_cross_topic_no_delivery_on_reconnect(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register(connection_port=CONNECTION_PORT)
        yield client.disconnect()
        yield client.send_notification(data=data, topic="Inbox")
        yield client.connect(connection_port=RP_CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(timeout=10)
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        yield client.ack(result["channelID"], result["version"])
        yield client.disconnect()
        yield client.connect(connection_port=CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(0.5)
        assert result is None
        yield client.disconnect()
        yield client.connect(connection_port=RP_CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(endpoint=41)
    def test_cross_topic_no_delivery_on_reconnect_reverse(self):
        data = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        yield client.send_notification(data=data, topic="Inbox")
        yield client.connect(connection_port=CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(timeout=10)
        # the following presumes that only `salt` is padded.
        clean_header = client._crypto_key.replace(
            '"', '').rstrip('=')
        assert result["headers"]["encryption"] == clean_header
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        yield client.ack(result["channelID"], result["version"])
        yield client.disconnect()
        yield client.connect(connection_port=RP_CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(0.5)
        assert result is None
        yield client.disconnect()
        yield client.connect(connection_port=CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(endpoint=10, conn=4)
    def test_cross_multiple_delivery_with_single_ack(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register(connection_port=CONNECTION_PORT)
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data)
        yield client.send_notification(data=data2)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] == base64url_encode(data)
        result2 = yield client.get_notification(timeout=0.5)
        assert result2 != {}
        assert result2["data"] == base64url_encode(data2)
        yield client.ack(result["channelID"], result["version"])

        yield client.disconnect()
        yield client.connect(connection_port=CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        result2 = yield client.get_notification()
        assert result2 != {}
        assert result2["data"] == base64url_encode(data2)
        yield client.ack(result["channelID"], result["version"])
        yield client.ack(result2["channelID"], result2["version"])

        # Verify no messages are delivered
        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(endpoint=45)
    def test_cross_multiple_delivery_with_single_ack_reverse(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data)
        yield client.send_notification(data=data2)
        yield client.connect(connection_port=CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] == base64url_encode(data)
        result2 = yield client.get_notification(timeout=0.5)
        assert result2 != {}
        assert result2["data"] == base64url_encode(data2)
        yield client.ack(result["channelID"], result["version"])

        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] == base64url_encode(data)
        assert result["messageType"] == "notification"
        result2 = yield client.get_notification()
        assert result2 != {}
        assert result2["data"] == base64url_encode(data2)
        yield client.ack(result["channelID"], result["version"])
        yield client.ack(result2["channelID"], result2["version"])

        # Verify no messages are delivered
        yield client.disconnect()
        yield client.connect(connection_port=CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(endpoint=43)
    def test_cross_multiple_delivery_with_multiple_ack(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register()
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data)
        yield client.send_notification(data=data2)
        yield client.connect(connection_port=CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        result2 = yield client.get_notification()
        assert result2 != {}
        assert result2["data"] in map(base64url_encode, [data, data2])
        yield client.ack(result2["channelID"], result2["version"])
        yield client.ack(result["channelID"], result["version"])

        yield client.disconnect()
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)

    @inlineCallbacks
    @max_logs(endpoint=10)
    def test_cross_multiple_delivery_with_multiple_ack_reverse(self):
        data = str(uuid.uuid4())
        data2 = str(uuid.uuid4())
        client = yield self.quick_register(connection_port=CONNECTION_PORT)
        yield client.disconnect()
        assert client.channels
        yield client.send_notification(data=data)
        yield client.send_notification(data=data2)
        yield client.connect()
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result != {}
        assert result["data"] in map(base64url_encode, [data, data2])
        result2 = yield client.get_notification()
        assert result2 != {}
        assert result2["data"] in map(base64url_encode, [data, data2])
        yield client.ack(result2["channelID"], result2["version"])
        yield client.ack(result["channelID"], result["version"])

        yield client.disconnect()
        yield client.connect(connection_port=CONNECTION_PORT)
        yield client.hello()
        result = yield client.get_notification(timeout=0.5)
        assert result is None
        yield self.shut_down(client)
