"""Rust Connection and Endpoint Node Integration Tests."""

import asyncio
import base64
import copy
import json
import logging
import os
import random
import signal
import socket
import string
import subprocess
import sys
import time
import uuid
from queue import Empty, Queue
from threading import Event, Thread
from typing import Any, AsyncGenerator, Generator, cast
from urllib.parse import urlparse

import ecdsa
import httpx
import psutil
import pytest
import uvicorn
import websockets
from cryptography.fernet import Fernet
from fastapi import FastAPI, Request
from jose import jws

from .async_push_test_client import AsyncPushTestClient, ClientMessageType

app = FastAPI()
logging.basicConfig(level=logging.DEBUG)
log = logging.getLogger(__name__)

here_dir = os.path.abspath(os.path.dirname(__file__))
tests_dir = os.path.dirname(here_dir)
root_dir = os.path.dirname(tests_dir)

SETUP_BT_SH = os.path.join(root_dir, "scripts", "setup_bt.sh")
BT_PROCESS: subprocess.Popen | None = None
BT_DB_SETTINGS: str | None = None

ROUTER_TABLE = os.environ.get("ROUTER_TABLE", "router_int_test")
MESSAGE_TABLE = os.environ.get("MESSAGE_TABLE", "message_int_test")
MSG_LIMIT = 20

CRYPTO_KEY = os.environ.get("CRYPTO_KEY") or Fernet.generate_key().decode("utf-8")
TRACKING_KEY = ecdsa.SigningKey.generate(curve=ecdsa.NIST256p)
TRACKING_PUB_KEY = cast(ecdsa.VerifyingKey, TRACKING_KEY.get_verifying_key())
CONNECTION_PORT = 9150
ENDPOINT_PORT = 9160
ROUTER_PORT = 9170
MP_CONNECTION_PORT = 9052
MP_ROUTER_PORT = 9072

CONNECTION_BINARY = os.environ.get("CONNECTION_BINARY", "autoconnect")
CONNECTION_SETTINGS_PREFIX = os.environ.get("CONNECTION_SETTINGS_PREFIX", "autoconnect__")

CN_SERVER: subprocess.Popen | None = None
CN_MP_SERVER: subprocess.Popen | None = None
EP_SERVER: subprocess.Popen | None = None
MOCK_SERVER_THREAD = None
CN_QUEUES: list = []
EP_QUEUES: list = []
STRICT_LOG_COUNTS = True

modules = [
    "autoconnect",
    "autoconnect_common",
    "autoconnect_web",
    "autoconnect_ws",
    "autoconnect_ws_sm",
    "autoendpoint",
    "autopush_common",
]
log_string = [f"{x}=trace" for x in modules]
RUST_LOG = ",".join(log_string) + ",error"


def get_free_port() -> int:
    """Get free port."""
    port: int
    s = socket.socket(socket.AF_INET, type=socket.SOCK_STREAM)
    s.bind(("127.0.0.1", 0))
    address, port = s.getsockname()
    s.close()
    return port


"""Try reading the database settings from the environment

If that points to a file, read the settings from that file.
"""


def get_db_settings() -> str | dict[str, str | int | float] | None:
    """Get database settings."""
    env_var = os.environ.get("DB_SETTINGS")
    if env_var:
        if os.path.isfile(env_var):
            with open(env_var, "r") as f:
                return f.read()
        return env_var
    return json.dumps(
        dict(
            router_table=ROUTER_TABLE,
            message_table=MESSAGE_TABLE,
            current_message_month=MESSAGE_TABLE,
            table_name="projects/test/instances/test/tables/autopush",
            router_family="router",
            message_family="message",
            message_topic_family="message_topic",
        )
    )


def base64url_encode(value: bytes | str) -> str:
    """Encode an unpadded Base64 URL-encoded string per RFC 7515."""
    if isinstance(value, str):
        value = bytes(value, "utf-8")

    return base64.urlsafe_b64encode(value).strip(b"=").decode("utf-8")


MOCK_SERVER_PORT: Any = get_free_port()
MOCK_MP_SERVICES: dict = {}
MOCK_MP_TOKEN: str = f"Bearer {uuid.uuid4().hex}"
MOCK_MP_POLLED: Event = Event()
MOCK_SENTRY_QUEUE: Queue = Queue()

"""Connection Server Config:
For local test debugging, set `AUTOPUSH_CN_CONFIG=_url_` to override
creation of the local server.
"""
CONNECTION_CONFIG: dict[str, Any] = dict(
    hostname="127.0.0.1",
    port=CONNECTION_PORT,
    endpoint_hostname="127.0.0.1",
    endpoint_port=ENDPOINT_PORT,
    router_port=ROUTER_PORT,
    endpoint_scheme="http",
    router_tablename=ROUTER_TABLE,
    message_tablename=MESSAGE_TABLE,
    crypto_key=f"[{CRYPTO_KEY}]",
    auto_ping_interval=30.0,
    auto_ping_timeout=10.0,
    close_handshake_timeout=5,
    max_connections=5000,
    human_logs="true",
    msg_limit=MSG_LIMIT,
    # new autoconnect
    db_dsn=os.environ.get("DB_DSN", "grpc://localhost:8086"),
    db_settings=get_db_settings(),
)

"""Connection Megaphone Config:
For local test debugging, set `AUTOPUSH_MP_CONFIG=_url_` to override
creation of the local server.
"""
MEGAPHONE_CONFIG: dict[str, str | int | float | None] = copy.deepcopy(CONNECTION_CONFIG)
MEGAPHONE_CONFIG.update(
    port=MP_CONNECTION_PORT,
    endpoint_port=ENDPOINT_PORT,
    router_port=MP_ROUTER_PORT,
    auto_ping_interval=0.5,
    auto_ping_timeout=10.0,
    close_handshake_timeout=5,
    max_connections=5000,
    megaphone_api_url=f"http://127.0.0.1:{MOCK_SERVER_PORT}/v1/broadcasts/",
    megaphone_api_token=MOCK_MP_TOKEN,
    megaphone_poll_interval=1,
)

"""Endpoint Server Config:
For local test debugging, set `AUTOPUSH_EP_CONFIG=_url_` to override
creation of the local server.
"""
ENDPOINT_CONFIG = dict(
    scheme="http",
    host="127.0.0.1",
    port=ENDPOINT_PORT,
    router_table_name=ROUTER_TABLE,
    message_table_name=MESSAGE_TABLE,
    human_logs="true",
    crypto_keys=f"[{CRYPTO_KEY}]",
    # convert to x692 format
    tracking_keys=f"[{base64.urlsafe_b64encode((b"\4" + TRACKING_PUB_KEY.to_string())).decode()}]",
)


def _get_vapid(
    key: ecdsa.SigningKey | None = None,
    payload: dict[str, Any] | None = None,
    endpoint: str | None = None,
) -> dict[str, str | bytes]:
    """Get VAPID information, including the `Authorization` header string,
    public and private keys.
    """
    global CONNECTION_CONFIG

    if endpoint is None:
        endpoint = (
            f"{CONNECTION_CONFIG.get("endpoint_scheme")}://"
            f"{CONNECTION_CONFIG.get("endpoint_hostname")}:"
            f"{CONNECTION_CONFIG.get("endpoint_port")}"
        )
    if not payload:
        payload = {
            "aud": endpoint,
            "exp": int(time.time()) + 86400,
            "sub": "mailto:admin@example.com",
        }
    if not payload.get("aud"):
        payload["aud"] = endpoint
    if not key:
        key = ecdsa.SigningKey.generate(curve=ecdsa.NIST256p)
    vk: ecdsa.VerifyingKey = cast(ecdsa.VerifyingKey, key.get_verifying_key())
    auth: str = jws.sign(payload, key, algorithm="ES256").strip("=")
    crypto_key: str = base64url_encode((b"\4" + vk.to_string()))
    return {"auth": auth, "crypto-key": crypto_key, "key": key}


def enqueue_output(out, queue) -> None:
    """Add lines from the out buffer to the provided queue."""
    for line in iter(out.readline, ""):
        queue.put(line)
    out.close()


def print_lines_in_queues(queues, prefix) -> None:
    """Print lines in queues to stdout."""
    for queue in queues:
        is_empty = False
        while not is_empty:
            try:
                line = queue.get_nowait()
            except Empty:
                is_empty = True
            else:
                sys.stdout.write(prefix + line)


@pytest.fixture
def fixture_max_conn_logs(request):
    """Fixture returning altered max_conn_logs value for process_logs fixture."""
    if hasattr(request, "param"):
        return int(request.param)


@pytest.fixture
def fixture_max_endpoint_logs(request):
    """Fixture returning altered max_endpoint_logs value for process_logs fixture."""
    if hasattr(request, "param"):
        return int(request.param)


@pytest.fixture(name="process_logs", autouse=True, scope="function")
async def fixture_process_logs(fixture_max_endpoint_logs, fixture_max_conn_logs):
    """Process (print) the testcase logs.

    Ensures a maximum level of logs allowed to be emitted when running
    w/ a `--release` mode connection/endpoint node

    Default of max_endpoint_logs=8 & max_conn_logs=3 for standard tests.
    Broadcast tests are set to max_endpoint_logs=4 & max_conn_logs=1.
    Values are modified for specific test cases to accommodate specific outputs.
    """
    yield

    # max_endpoint_logs and max_conn_logs are max log lines
    # allowed to be emitted by each node type
    max_endpoint_logs: int = fixture_max_endpoint_logs or 8
    max_conn_logs: int = fixture_max_conn_logs or 3

    conn_count = sum(queue.qsize() for queue in CN_QUEUES)
    endpoint_count = sum(queue.qsize() for queue in EP_QUEUES)

    print_lines_in_queues(CN_QUEUES, f"{CONNECTION_BINARY.upper()}: ")
    print_lines_in_queues(EP_QUEUES, "AUTOENDPOINT: ")

    print("🐍🟢 MAX_CONN_LOGS", max_conn_logs)
    print("🐍🟢 MAX_ENDPOINT_LOGS", max_endpoint_logs)

    if not STRICT_LOG_COUNTS:
        return

    msg = "endpoint node emitted excessive log statements, count: {} > max: {}"
    # Give an extra to endpoint for potential startup log messages
    # (e.g. when running tests individually)
    max_endpoint_logs += 1
    assert endpoint_count <= max_endpoint_logs, msg.format(endpoint_count, max_endpoint_logs)

    msg = "conn node emitted excessive log statements, count: {} > max: {}"
    assert conn_count <= max_conn_logs, msg.format(conn_count, max_conn_logs)


@app.get("/v1/broadcasts")
async def broadcast_handler(request: Request) -> dict[str, dict[str, str]]:
    """Broadcast handler setup."""
    assert request.headers["Authorization"] == MOCK_MP_TOKEN
    MOCK_MP_POLLED.set()
    return dict(broadcasts=MOCK_MP_SERVICES)


@app.post("/api/1/envelope/")
async def sentry_handler(request: Request) -> dict[str, str]:
    """Sentry handler configuration."""
    _headers, _item_headers, payload = (await request.body()).splitlines()
    MOCK_SENTRY_QUEUE.put(json.loads(payload))
    return {"id": "fc6d8c0c43fc4630ad850ee518f1b9d0"}


def kill_process(process) -> None:
    """Kill child processes."""
    # This kinda sucks, but its the only way to nuke the child procs
    if process is None:
        return
    proc = psutil.Process(pid=process.pid)
    child_procs = proc.children(recursive=True)
    for p in [proc] + child_procs:
        os.kill(p.pid, signal.SIGTERM)
    process.wait()


def get_rust_binary_path(binary) -> str:
    """Get path to pre-built Rust binary.
    This presumes that the application has already been built with proper features.
    """
    global STRICT_LOG_COUNTS

    rust_bin: str = root_dir + f"/target/release/{binary}"
    possible_paths: list[str] = [
        f"/target/debug/{binary}",
        f"/{binary}/target/release/{binary}",
        f"/{binary}/target/debug/{binary}",
    ]
    while possible_paths and not os.path.exists(rust_bin):  # pragma: nocover
        rust_bin = root_dir + possible_paths.pop(0)

    if "release" not in rust_bin:
        # disable checks for chatty debug mode binaries
        STRICT_LOG_COUNTS = False

    return rust_bin


def write_config_to_env(config, prefix) -> None:
    """Write configurations to application read environment variables."""
    for key, val in config.items():
        new_key = prefix + key
        log.debug(f"✍ config {new_key} => {val}")
        os.environ[new_key.upper()] = str(val)


def capture_output_to_queue(output_stream) -> Queue:
    """Capture output to log queue."""
    log_queue: Queue = Queue()
    t = Thread(target=enqueue_output, args=(output_stream, log_queue))
    t.daemon = True  # thread dies with the program
    t.start()
    return log_queue


def setup_bt() -> None:
    """Set up BigTable emulator."""
    global BT_PROCESS, BT_DB_SETTINGS
    log.debug("🐍🟢 Starting bigtable emulator")
    # All calls to subprocess module for setup. Not passing untrusted input.
    # Will look at future replacement when moving to Docker.
    # https://bandit.readthedocs.io/en/latest/plugins/b603_subprocess_without_shell_equals_true.html
    BT_PROCESS = subprocess.Popen("gcloud beta emulators bigtable start".split(" "))  # nosec
    os.environ["BIGTABLE_EMULATOR_HOST"] = "127.0.0.1:8086"
    try:
        BT_DB_SETTINGS = os.environ.get(
            "BT_DB_SETTINGS",
            json.dumps(
                {
                    "table_name": "projects/test/instances/test/tables/autopush",
                }
            ),
        )
        # Note: This will produce an emulator that runs on DB_DSN="grpc://localhost:8086"
        # using a Table Name of "projects/test/instances/test/tables/autopush"
        log.debug("🐍🟢 Setting up bigtable")
        vv = subprocess.call([SETUP_BT_SH])  # nosec
        log.debug(vv)
    except Exception as e:
        log.error("Bigtable Setup Error {}", e)
        raise


def run_fastapi_app(host, port) -> None:
    """Run FastAPI app with uvicorn."""
    uvicorn.run(app, host=host, port=port, log_level="debug")


def setup_mock_server() -> None:
    """Set up mock server."""
    global MOCK_SERVER_THREAD

    MOCK_SERVER_THREAD = Thread(
        target=run_fastapi_app, kwargs=dict(host="127.0.0.1", port=MOCK_SERVER_PORT)
    )
    MOCK_SERVER_THREAD.daemon = True
    MOCK_SERVER_THREAD.start()

    # Sentry API mock
    os.environ["SENTRY_DSN"] = f"http://foo:bar@127.0.0.1:{MOCK_SERVER_PORT}/1"


def setup_connection_server(connection_binary) -> None:
    """Set up connection server from config."""
    global CN_SERVER, BT_PROCESS

    # NOTE:
    # due to a change in Config, autopush uses a double
    # underscore as a separator (e.g. "AUTOEND__FCM__MIN_TTL" ==
    # `settings.fcm.min_ttl`)
    url = os.getenv("AUTOPUSH_CN_SERVER")
    if url is not None:
        parsed = urlparse(url)
        CONNECTION_CONFIG["hostname"] = parsed.hostname
        CONNECTION_CONFIG["port"] = parsed.port
        CONNECTION_CONFIG["endpoint_scheme"] = parsed.scheme
        write_config_to_env(CONNECTION_CONFIG, CONNECTION_SETTINGS_PREFIX)
        log.debug("Using existing Connection server")
        return
    else:
        write_config_to_env(CONNECTION_CONFIG, CONNECTION_SETTINGS_PREFIX)
    cmd = [connection_binary]
    run_args = os.getenv("RUN_ARGS")
    if run_args is not None:
        cmd.append(run_args)
    log.debug(f"🐍🟢 Starting Connection server: {' '.join(cmd)}")
    CN_SERVER = subprocess.Popen(
        cmd,
        shell=True,
        env=os.environ,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        universal_newlines=True,
    )  # nosec

    # Spin up the readers to dump the output from stdout/stderr
    out_q = capture_output_to_queue(CN_SERVER.stdout)
    err_q = capture_output_to_queue(CN_SERVER.stderr)
    CN_QUEUES.extend([out_q, err_q])


def setup_megaphone_server(connection_binary) -> None:
    """Set up megaphone server from configuration."""
    global CN_MP_SERVER

    url = os.getenv("AUTOPUSH_MP_SERVER")
    if url is not None:
        parsed = urlparse(url)
        MEGAPHONE_CONFIG["hostname"] = parsed.hostname
        MEGAPHONE_CONFIG["port"] = parsed.port
        MEGAPHONE_CONFIG["endpoint_scheme"] = parsed.scheme
        url = os.getenv("AUTOPUSH_EP_SERVER")
        if url is not None:
            parsed = urlparse(url)
            MEGAPHONE_CONFIG["endpoint_port"] = parsed.port
        write_config_to_env(MEGAPHONE_CONFIG, CONNECTION_SETTINGS_PREFIX)
        log.debug("Using existing Megaphone server")
        return
    else:
        write_config_to_env(MEGAPHONE_CONFIG, CONNECTION_SETTINGS_PREFIX)
    cmd = [connection_binary]
    log.debug(f"🐍🟢 Starting Megaphone server: {' '.join(cmd)}")
    CN_MP_SERVER = subprocess.Popen(cmd, shell=True, env=os.environ)  # nosec


def setup_endpoint_server() -> None:
    """Set up endpoint server from configuration."""
    global CONNECTION_CONFIG, EP_SERVER, BT_PROCESS, ENDPOINT_CONFIG

    # Set up environment
    # NOTE:
    # due to a change in Config, autoendpoint uses a double
    # underscore as a separator (e.g. "AUTOEND__FCM__MIN_TTL" ==
    # `settings.fcm.min_ttl`)
    url = os.getenv("AUTOPUSH_EP_SERVER")
    ENDPOINT_CONFIG["db_dsn"] = CONNECTION_CONFIG["db_dsn"]
    ENDPOINT_CONFIG["db_settings"] = CONNECTION_CONFIG["db_settings"]
    if url is not None:
        parsed = urlparse(url)
        ENDPOINT_CONFIG["hostname"] = parsed.hostname
        ENDPOINT_CONFIG["port"] = parsed.port
        ENDPOINT_CONFIG["endpoint_scheme"] = parsed.scheme
        log.debug("Using existing Endpoint server")
        return
    else:
        write_config_to_env(ENDPOINT_CONFIG, "autoend__")

    # Run autoendpoint
    cmd = [get_rust_binary_path("autoendpoint")]

    log.debug(f"🐍🟢 Starting Endpoint server: {' '.join(cmd)}")
    EP_SERVER = subprocess.Popen(
        cmd,
        shell=True,
        env=os.environ,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        universal_newlines=True,
    )  # nosec

    # Spin up the readers to dump the output from stdout/stderr
    out_q = capture_output_to_queue(EP_SERVER.stdout)
    err_q = capture_output_to_queue(EP_SERVER.stderr)
    EP_QUEUES.extend([out_q, err_q])


@pytest.fixture(autouse=True, scope="session")
def setup_teardown() -> Generator:
    """Set up and tear down test resources within module.

    Setup: BigTable and connection, endpoint, mock and megaphone servers.
    """
    global CN_SERVER, CN_QUEUES, CN_MP_SERVER, MOCK_SERVER_THREAD, STRICT_LOG_COUNTS, RUST_LOG

    if "SKIP_INTEGRATION" in os.environ:  # pragma: nocover
        log.debug("Skipping integration tests")
        pytest.skip("Skipping integration tests", allow_module_level=True)

    if CONNECTION_CONFIG.get("db_dsn", "").startswith("grpc"):
        log.debug("Setting up BigTable")
        setup_bt()

    setup_mock_server()

    # NOTE: This sleep is required to ensure that autopush does not ping the Megaphone
    # v1/broadcasts endpoint before it is ready.
    time.sleep(1)

    log.debug(f"🐍🟢 Rust Log: {RUST_LOG}")
    os.environ["RUST_LOG"] = RUST_LOG
    connection_binary = get_rust_binary_path(CONNECTION_BINARY)
    setup_connection_server(connection_binary)
    setup_megaphone_server(connection_binary)
    setup_endpoint_server()
    time.sleep(2)

    yield

    if BT_PROCESS:
        os.unsetenv("BIGTABLE_EMULATOR_HOST")
        log.debug("🐍🔴 Stopping bigtable")
        kill_process(BT_PROCESS)
    log.debug("🐍🔴 Stopping connection server")
    kill_process(CN_SERVER)
    log.debug("🐍🔴 Stopping megaphone server")
    kill_process(CN_MP_SERVER)
    log.debug("🐍🔴 Stopping endpoint server")
    kill_process(EP_SERVER)


@pytest.fixture(name="ws_url", scope="session")
def fixture_ws_url() -> str:
    """Return defined url for websocket connection."""
    return f"ws://localhost:{CONNECTION_PORT}/"


@pytest.fixture(name="broadcast_ws_url", scope="session")
def fixture_broadcast_ws_url() -> str:
    """Return websocket url for megaphone broadcast testing."""
    return f"ws://localhost:{MP_CONNECTION_PORT}/"


@pytest.fixture(name="vapid_payload", scope="session")
def fixture_vapid_payload() -> dict[str, int | str]:
    """Return a test fixture of a vapid payload."""
    return {
        "exp": int(time.time()) + 86400,
        "sub": "mailto:admin@example.com",
    }


@pytest.fixture(name="clear_sentry_queue", autouse=True, scope="function")
def fixture_clear_sentry_queue() -> Generator:
    """Clear any values present in Sentry queue.
    Scoped to run automatically after each test function.
    """
    yield
    while not MOCK_SENTRY_QUEUE.empty():
        MOCK_SENTRY_QUEUE.get_nowait()


@pytest.fixture(name="ws_config", scope="session")
def fixture_ws_config() -> dict[str, int]:
    """Return collection of configuration values."""
    return {"connection_port": 9150}


@pytest.fixture(name="test_client", scope="function")
async def fixture_test_client(ws_url) -> AsyncGenerator:
    """Return a push test client for use within tests that can have
    a custom websocket url passed in to initialize.
    """
    client = AsyncPushTestClient(ws_url)
    yield client
    if client.ws:
        log.debug(f"🐍#### test client at {client.url} disconnected.")
        await client.disconnect()


@pytest.fixture(name="test_client_broadcast", scope="function")
async def fixture_test_client_broadcast(broadcast_ws_url) -> AsyncGenerator:
    """Return a push test client for use within tests that can have
    a custom websocket url passed in to initialize.
    """
    client = AsyncPushTestClient(broadcast_ws_url)
    yield client
    await client.disconnect()


@pytest.fixture(name="registered_test_client", scope="function")
async def fixture_registered_test_client(ws_config) -> AsyncGenerator:
    """Perform a connection initialization, which includes a new connection,
    `hello`, and channel registration.
    """
    log.debug(f"🐍#### Connecting to ws://localhost:{ws_config['connection_port']}/")
    client = AsyncPushTestClient(f"ws://localhost:{ws_config['connection_port']}/")
    await client.connect()
    await client.hello()
    await client.register()
    log.debug("🐍 Test Client Connected and Registered")
    yield client
    await client.disconnect()
    log.debug("🐍 Test Client Disconnected")


@pytest.mark.parametrize("fixture_max_conn_logs", [4], indirect=True)
async def test_sentry_output_autoconnect(test_client: AsyncPushTestClient) -> None:
    """Test sentry output for autoconnect."""
    if os.getenv("SKIP_SENTRY"):
        log.debug("Skipping test_sentry_output_autoconnect")
        pytest.skip("Skipping test_sentry_output_autoconnect")
    # Ensure bad data doesn't throw errors
    await test_client.connect()
    await test_client.hello()
    await test_client.send_bad_data()
    await test_client.disconnect()

    # LogCheck does throw an error every time
    async with httpx.AsyncClient() as httpx_client:
        await httpx_client.get(f"http://127.0.0.1:{CONNECTION_PORT}/v1/err/crit", timeout=30)

    event1 = MOCK_SENTRY_QUEUE.get(timeout=5)
    # NOTE: this timeout increased to 5 seconds as was yielding
    # errors in CI due to potential race condition and lingering
    # artifact in the MOCK_SENTRY_QUEUE. It could cause test_sentry_output_autoendpoint
    # to have two LogCheck entires and test_no_sentry_output would have data
    # when it should not.

    # new autoconnect emits 2 events
    try:
        MOCK_SENTRY_QUEUE.get(timeout=5)
    except Empty:
        pass
    assert event1["exception"]["values"][0]["value"] == "LogCheck"


@pytest.mark.parametrize("fixture_max_endpoint_logs", [1], indirect=True)
async def test_sentry_output_autoendpoint(registered_test_client: AsyncPushTestClient) -> None:
    """Test sentry output for autoendpoint."""
    if os.getenv("SKIP_SENTRY"):
        log.debug("Skipping test_sentry_output_autoendpoint")
        pytest.skip("Skipping test_sentry_output_autoendpoint")
    endpoint = registered_test_client.get_host_client_endpoint()
    await registered_test_client.disconnect()
    async with httpx.AsyncClient() as httpx_client:
        await httpx_client.get(f"{endpoint}/__error__", timeout=5)
        # 2 events excpted: 1 from a panic and 1 from a returned Error

    event1 = MOCK_SENTRY_QUEUE.get(timeout=5)
    event2 = MOCK_SENTRY_QUEUE.get(timeout=5)
    values = (
        event1["exception"]["values"][0]["value"],
        event2["exception"]["values"][0]["value"],
    )
    # "ERROR:Success" & "LogCheck" are the commonly expected values.
    # When updating to async/await, noted cases in CI where only LogCheck
    # occured, likely due to a race condition.
    # Establishing a single check for "LogCheck" as sufficent guarantee,
    # since Sentry is capturing emission
    assert "LogCheck" in values
    assert sorted(values) == ["ERROR:Success", "LogCheck"]


@pytest.mark.parametrize("fixture_max_conn_logs", [4], indirect=True)
async def test_no_sentry_output(ws_url: str) -> None:
    """Test for no Sentry output."""
    if os.getenv("SKIP_SENTRY"):
        log.debug("Skipping test_no_sentry_output")
        pytest.skip("Skipping test_no_sentry_output")

    ws_url = urlparse(ws_url)._replace(scheme="http").geturl()

    try:
        async with httpx.AsyncClient() as httpx_client:
            await httpx_client.get(ws_url, timeout=30)
            data = MOCK_SENTRY_QUEUE.get(timeout=5)
            assert not data
    except httpx.ConnectError:
        pass
    except Empty:
        pass


async def test_hello_echo(test_client: AsyncPushTestClient) -> None:
    """Test hello echo."""
    await test_client.connect()
    result = await test_client.hello()
    assert result != {}
    assert result["use_webpush"] is True


async def test_hello_with_bad_prior_uaid(test_client: AsyncPushTestClient) -> None:
    """Test hello with bad prior uaid."""
    non_uaid: str = uuid.uuid4().hex
    await test_client.connect()
    result = await test_client.hello(uaid=non_uaid)
    assert result != {}
    assert result["uaid"] != non_uaid
    assert result["use_webpush"] is True


async def test_basic_delivery(registered_test_client: AsyncPushTestClient) -> None:
    """Test basic regular push message delivery."""
    uuid_data: str = str(uuid.uuid4())
    result = await registered_test_client.send_notification(data=uuid_data)
    # the following presumes that only `salt` is padded.
    clean_header = registered_test_client._crypto_key.replace('"', "").rstrip("=")
    assert result["headers"]["encryption"] == clean_header
    assert result["data"] == base64url_encode(bytes(uuid_data, "utf-8"))
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value


async def test_topic_basic_delivery(registered_test_client: AsyncPushTestClient) -> None:
    """Test basic topic push message delivery."""
    uuid_data: str = str(uuid.uuid4())
    result = await registered_test_client.send_notification(data=uuid_data, topic="Inbox")
    # the following presumes that only `salt` is padded.
    clean_header = registered_test_client._crypto_key.replace('"', "").rstrip("=")
    assert result["headers"]["encryption"] == clean_header
    assert result["data"] == base64url_encode(uuid_data)
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value


async def test_topic_replacement_delivery(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that a topic push message replaces it's prior version."""
    uuid_data_1: str = str(uuid.uuid4())
    uuid_data_2: str = str(uuid.uuid4())
    await registered_test_client.disconnect()
    await registered_test_client.send_notification(data=uuid_data_1, topic="Inbox", status=201)
    await registered_test_client.send_notification(data=uuid_data_2, topic="Inbox", status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    log.debug("get_notification result:", result)
    # the following presumes that only `salt` is padded.
    clean_header = registered_test_client._crypto_key.replace('"', "").rstrip("=")
    assert result["headers"]["encryption"] == clean_header
    assert result["data"] == base64url_encode(uuid_data_2)
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value
    result = await registered_test_client.get_notification()
    assert result is None


@pytest.mark.parametrize("fixture_max_conn_logs", [4], indirect=True)
async def test_topic_no_delivery_on_reconnect(registered_test_client: AsyncPushTestClient) -> None:
    """Test that a topic message does not attempt to redeliver on reconnect."""
    uuid_data: str = str(uuid.uuid4())
    await registered_test_client.disconnect()
    await registered_test_client.send_notification(data=uuid_data, topic="Inbox", status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=10)
    # the following presumes that only `salt` is padded.
    clean_header = registered_test_client._crypto_key.replace('"', "").rstrip("=")
    assert result["headers"]["encryption"] == clean_header
    assert result["data"] == base64url_encode(uuid_data)
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value
    await registered_test_client.ack(result["channelID"], result["version"])
    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    assert result is None
    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()


async def test_basic_delivery_with_vapid(
    registered_test_client: AsyncPushTestClient,
    vapid_payload: dict[str, int | str],
) -> None:
    """Test delivery of a basic push message with a VAPID header."""
    uuid_data: str = str(uuid.uuid4())
    vapid_info = _get_vapid(payload=vapid_payload)
    result = await registered_test_client.send_notification(data=uuid_data, vapid=vapid_info)
    # the following presumes that only `salt` is padded.
    clean_header = registered_test_client._crypto_key.replace('"', "").rstrip("=")
    assert result["headers"]["encryption"] == clean_header
    assert result["data"] == base64url_encode(uuid_data)
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value
    # The key we used should not have been registered, so no tracking should
    # be occurring.
    log.debug(f"🔍 Reliability: {result.get("reliability_id")}")
    assert result.get("reliability_id") is None


async def test_basic_delivery_with_tracked_vapid(
    registered_test_client: AsyncPushTestClient,
    vapid_payload: dict[str, int | str],
) -> None:
    """Test delivery of a basic push message with a VAPID header."""
    uuid_data: str = str(uuid.uuid4())
    vapid_info = _get_vapid(key=TRACKING_KEY, payload=vapid_payload)
    # quick sanity check to ensure that the keys match.
    # (ideally, this should dump as x962, but DER is good enough.)
    key = cast(
        ecdsa.VerifyingKey, cast(ecdsa.SigningKey, vapid_info["key"]).get_verifying_key()
    ).to_der()

    assert key == TRACKING_PUB_KEY.to_der()

    # let's do an offline submit so we can validate the reliability_id survives storage.
    await registered_test_client.disconnect()
    await registered_test_client.send_notification(data=uuid_data, vapid=vapid_info)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()

    # the following presumes that only `salt` is padded.
    clean_header = registered_test_client._crypto_key.replace('"', "").rstrip("=")
    assert result["headers"]["encryption"] == clean_header
    assert result["data"] == base64url_encode(uuid_data)
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value
    log.debug(f"🔍 reliability {result}")
    assert result["reliability_id"] is not None


async def test_basic_delivery_with_invalid_vapid(
    registered_test_client: AsyncPushTestClient,
    vapid_payload: dict[str, int | str],
) -> None:
    """Test basic delivery with invalid VAPID header."""
    uuid_data: str = str(uuid.uuid4())
    vapid_info = _get_vapid(
        payload=vapid_payload, endpoint=registered_test_client.get_host_client_endpoint()
    )
    vapid_info["crypto-key"] = "invalid"
    await registered_test_client.send_notification(data=uuid_data, vapid=vapid_info, status=401)


async def test_basic_delivery_with_invalid_vapid_exp(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test basic delivery of a push message with invalid VAPID `exp` assertion."""
    uuid_data: str = str(uuid.uuid4())
    vapid_info = _get_vapid(
        payload={
            "aud": registered_test_client.get_host_client_endpoint(),
            "exp": "@",
            "sub": "mailto:admin@example.com",
        }
    )
    vapid_info["crypto-key"] = "invalid"
    await registered_test_client.send_notification(data=uuid_data, vapid=vapid_info, status=401)


async def test_basic_delivery_with_invalid_vapid_auth(
    registered_test_client: AsyncPushTestClient,
    vapid_payload: dict[str, int | str],
) -> None:
    """Test basic delivery with invalid VAPID auth."""
    uuid_data: str = str(uuid.uuid4())
    vapid_info = _get_vapid(
        payload=vapid_payload,
        endpoint=registered_test_client.get_host_client_endpoint(),
    )
    vapid_info["auth"] = ""
    await registered_test_client.send_notification(data=uuid_data, vapid=vapid_info, status=401)


async def test_basic_delivery_with_invalid_signature(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that a basic delivery with invalid VAPID signature fails."""
    uuid_data: str = str(uuid.uuid4())
    vapid_info = _get_vapid(
        payload={
            "aud": registered_test_client.get_host_client_endpoint(),
            "sub": "mailto:admin@example.com",
        }
    )
    vapid_info["auth"] = f"{vapid_info['auth'][:-3]!r}bad"
    await registered_test_client.send_notification(data=uuid_data, vapid=vapid_info, status=401)


async def test_basic_delivery_with_invalid_vapid_ckey(
    registered_test_client: AsyncPushTestClient,
    vapid_payload: dict[str, int | str],
) -> None:
    """Test that basic delivery with invalid VAPID crypto-key fails."""
    uuid_data: str = str(uuid.uuid4())
    vapid_info = _get_vapid(
        payload=vapid_payload, endpoint=registered_test_client.get_host_client_endpoint()
    )
    vapid_info["crypto-key"] = "invalid|"
    await registered_test_client.send_notification(data=uuid_data, vapid=vapid_info, status=401)


async def test_delivery_repeat_without_ack(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that message delivery repeats if the client does not acknowledge messages."""
    uuid_data: str = str(uuid.uuid4())
    await registered_test_client.disconnect()
    assert registered_test_client.channels
    await registered_test_client.send_notification(data=uuid_data, status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    assert result is not None
    assert result["data"] == base64url_encode(uuid_data)

    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    assert result != {}
    assert result["data"] == base64url_encode(uuid_data)


async def test_repeat_delivery_with_disconnect_without_ack(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that message delivery repeats if the client disconnects
    without acknowledging the message.
    """
    uuid_data: str = str(uuid.uuid4())
    result = await registered_test_client.send_notification(data=uuid_data)
    assert result != {}
    assert result["data"] == base64url_encode(uuid_data)
    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    assert result != {}
    assert result["data"] == base64url_encode(uuid_data)


async def test_multiple_delivery_repeat_without_ack(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that the server will always try to deliver messages
    until the client acknowledges them.
    """
    uuid_data_1: str = str(uuid.uuid4())
    uuid_data_2: str = str(uuid.uuid4())
    await registered_test_client.disconnect()
    assert registered_test_client.channels
    await registered_test_client.send_notification(data=uuid_data_1, status=201)
    await registered_test_client.send_notification(data=uuid_data_2, status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    assert result != {}
    assert result["data"] in map(base64url_encode, [uuid_data_1, uuid_data_2])
    result = await registered_test_client.get_notification()
    assert result != {}
    assert result["data"] in map(base64url_encode, [uuid_data_1, uuid_data_2])

    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    assert result != {}
    assert result["data"] in map(base64url_encode, [uuid_data_1, uuid_data_2])
    result = await registered_test_client.get_notification()
    assert result != {}
    assert result["data"] in map(base64url_encode, [uuid_data_1, uuid_data_2])


async def test_topic_expired(registered_test_client: AsyncPushTestClient) -> None:
    """Test that the server will not deliver a message topic that has expired."""
    uuid_data: str = str(uuid.uuid4())
    await registered_test_client.disconnect()
    assert registered_test_client.channels
    await registered_test_client.send_notification(data=uuid_data, ttl=1, topic="test", status=201)
    await registered_test_client.sleep(2)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result is None
    result = await registered_test_client.send_notification(data=uuid_data, topic="test")
    assert result != {}
    assert result["data"] == base64url_encode(uuid_data)


@pytest.mark.parametrize("fixture_max_conn_logs", [4], indirect=True)
async def test_multiple_delivery_with_single_ack(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that the server provides the right unacknowledged messages
    if the client only acknowledges one of the received messages.
    Note: the `data` fields are constructed so that they return
    `FirstMessage` and `OtherMessage`, which may be useful for debugging.
    """
    uuid_data_1: bytes = (
        b"\x16*\xec\xb4\xc7\xac\xb1\xa8\x1e" + str(uuid.uuid4()).encode()
    )  # FirstMessage
    uuid_data_2: bytes = (
        b":\xd8^\xac\xc7\xac\xb1\xa8\x1e" + str(uuid.uuid4()).encode()
    )  # OtherMessage
    await registered_test_client.disconnect()
    assert registered_test_client.channels
    await registered_test_client.send_notification(data=uuid_data_1, status=201)
    await registered_test_client.send_notification(data=uuid_data_2, status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result != {}
    assert result["data"] == base64url_encode(uuid_data_1)
    result2 = await registered_test_client.get_notification(timeout=0.5)
    assert result2 != {}
    assert result2["data"] == base64url_encode(uuid_data_2)
    await registered_test_client.ack(result["channelID"], result["version"])

    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result != {}
    assert result["data"] == base64url_encode(uuid_data_1)
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value
    result2 = await registered_test_client.get_notification()
    assert result2 != {}
    assert result2["data"] == base64url_encode(uuid_data_2)
    await registered_test_client.ack(result["channelID"], result["version"])
    await registered_test_client.ack(result2["channelID"], result2["version"])

    # Verify no messages are delivered
    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result is None


async def test_multiple_delivery_with_multiple_ack(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that the server provides the no additional unacknowledged messages
    if the client acknowledges both of the received messages.
    Note: the `data` fields are constructed so that they return
    `FirstMessage` and `OtherMessage`, which may be useful for debugging.
    """
    uuid_data_1: bytes = (
        b"\x16*\xec\xb4\xc7\xac\xb1\xa8\x1e" + str(uuid.uuid4()).encode()
    )  # FirstMessage
    uuid_data_2: bytes = (
        b":\xd8^\xac\xc7\xac\xb1\xa8\x1e" + str(uuid.uuid4()).encode()
    )  # OtherMessage
    await registered_test_client.disconnect()
    assert registered_test_client.channels
    await registered_test_client.send_notification(data=uuid_data_1, status=201)
    await registered_test_client.send_notification(data=uuid_data_2, status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result != {}
    assert result["data"] in map(base64url_encode, [uuid_data_1, uuid_data_2])
    log.debug(f"🟩🟩 Result:: {result['data']}")
    result2 = await registered_test_client.get_notification()
    assert result2 != {}
    assert result2["data"] in map(base64url_encode, [uuid_data_1, uuid_data_2])
    await registered_test_client.ack(result2["channelID"], result2["version"])
    await registered_test_client.ack(result["channelID"], result["version"])

    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result is None


async def test_no_delivery_to_unregistered(registered_test_client: AsyncPushTestClient) -> None:
    """Test that the server does not try to deliver to unregistered channel IDs."""
    uuid_data: str = str(uuid.uuid4())
    assert registered_test_client.channels
    chan = list(registered_test_client.channels.keys())[0]

    result = await registered_test_client.send_notification(data=uuid_data)
    assert result["channelID"] == chan
    assert result["data"] == base64url_encode(uuid_data)
    await registered_test_client.ack(result["channelID"], result["version"])

    await registered_test_client.unregister(chan)
    result = await registered_test_client.send_notification(data=uuid_data, status=410)

    # Verify cache-control
    notif_response_headers = registered_test_client.notif_response.headers  # type: ignore
    assert notif_response_headers.get("Cache-Control") == "max-age=86400"

    assert result is None


async def test_ttl_0_connected(registered_test_client: AsyncPushTestClient) -> None:
    """Test that a message with a TTL=0 is delivered to a client that is actively connected."""
    uuid_data: str = str(uuid.uuid4())
    result = await registered_test_client.send_notification(data=uuid_data, ttl=0)
    assert result is not None
    # the following presumes that only `salt` is padded.
    clean_header = registered_test_client._crypto_key.replace('"', "").rstrip("=")
    assert result["headers"]["encryption"] == clean_header
    assert result["data"] == base64url_encode(uuid_data)
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value


async def test_ttl_0_not_connected(registered_test_client: AsyncPushTestClient) -> None:
    """Test that a message with a TTL=0 and a recipient client that is not connected,
    is not delivered when the client reconnects.
    """
    uuid_data: str = str(uuid.uuid4())
    await registered_test_client.disconnect()
    await registered_test_client.send_notification(data=uuid_data, ttl=0, status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result is None


async def test_ttl_expired(registered_test_client: AsyncPushTestClient) -> None:
    """Test that messages with a TTL that has expired are not delivered
    to a recipient client.
    """
    uuid_data: str = str(uuid.uuid4())
    await registered_test_client.disconnect()
    await registered_test_client.send_notification(data=uuid_data, ttl=1, status=201)
    await asyncio.sleep(1)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result is None


@pytest.mark.parametrize("fixture_max_endpoint_logs", [28], indirect=True)
async def test_ttl_batch_expired_and_good_one(registered_test_client: AsyncPushTestClient) -> None:
    """Test that if a batch of messages are received while the recipient is offline,
    only messages that have not expired are sent to the recipient.
    This test checks if the latest pending message is not expired.
    """
    uuid_data_1: bytes = str(uuid.uuid4()).encode()
    uuid_data_2: bytes = base64.urlsafe_b64decode("0012") + str(uuid.uuid4()).encode()
    await registered_test_client.disconnect()
    for x in range(0, 12):
        prefix = base64.urlsafe_b64decode(f"{x:04d}")
        await registered_test_client.send_notification(
            data=prefix + uuid_data_1, ttl=1, status=201
        )

    await registered_test_client.send_notification(data=uuid_data_2, status=201)
    await asyncio.sleep(1)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification(timeout=4)
    assert result is not None
    # the following presumes that only `salt` is padded.
    clean_header = registered_test_client._crypto_key.replace('"', "").rstrip("=")
    assert result["headers"]["encryption"] == clean_header
    assert result["data"] == base64url_encode(uuid_data_2)
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value
    result = await registered_test_client.get_notification(timeout=0.5)
    assert result is None


@pytest.mark.parametrize("fixture_max_endpoint_logs", [28], indirect=True)
async def test_ttl_batch_partly_expired_and_good_one(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that if a batch of messages are received while the recipient is offline,
    only messages that have not expired are sent to the recipient.
    This test checks if there is an equal mix of expired and unexpired messages.
    """
    uuid_data_1: str = str(uuid.uuid4())
    uuid_data_2: str = str(uuid.uuid4())
    uuid_data_3: str = str(uuid.uuid4())
    await registered_test_client.disconnect()
    for x in range(0, 6):
        await registered_test_client.send_notification(data=uuid_data_1, status=201)

    for x in range(0, 6):
        await registered_test_client.send_notification(data=uuid_data_2, ttl=1, status=201)

    await registered_test_client.send_notification(data=uuid_data_3, status=201)
    await asyncio.sleep(1)
    await registered_test_client.connect()
    await registered_test_client.hello()

    # Pull out and ack the first
    for x in range(0, 6):
        result = await registered_test_client.get_notification(timeout=4)
        assert result is not None
        assert result["data"] == base64url_encode(uuid_data_1)
        await registered_test_client.ack(result["channelID"], result["version"])

    # Should have one more that is uuid_data_3, this will only arrive if the
    # other six were acked as that hits the batch size
    result = await registered_test_client.get_notification(timeout=4)
    assert result is not None
    assert result["data"] == base64url_encode(uuid_data_3)

    # No more
    result = await registered_test_client.get_notification()
    assert result is None


async def test_message_without_crypto_headers(registered_test_client: AsyncPushTestClient) -> None:
    """Test that a message without crypto headers, but has data is not accepted."""
    uuid_data: str = str(uuid.uuid4())
    result = await registered_test_client.send_notification(
        data=uuid_data, use_header=False, status=400
    )
    assert result is None


async def test_empty_message_without_crypto_headers(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that a message without crypto headers, and does not have data, is accepted."""
    result = await registered_test_client.send_notification(use_header=False)
    assert result is not None
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value
    assert "headers" not in result
    assert "data" not in result
    await registered_test_client.ack(result["channelID"], result["version"])

    await registered_test_client.disconnect()
    await registered_test_client.send_notification(use_header=False, status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    assert result is not None
    assert "headers" not in result
    assert "data" not in result
    await registered_test_client.ack(result["channelID"], result["version"])


async def test_empty_message_with_crypto_headers(
    registered_test_client: AsyncPushTestClient,
) -> None:
    """Test that an empty message with crypto headers does not send either `headers`
    or `data` as part of the incoming websocket `notification` message.
    """
    result = await registered_test_client.send_notification()
    assert result is not None
    assert result["messageType"] == ClientMessageType.NOTIFICATION.value
    assert "headers" not in result
    assert "data" not in result

    result2 = await registered_test_client.send_notification()
    # We shouldn't store headers for blank messages.
    assert result2 is not None
    assert result2["messageType"] == ClientMessageType.NOTIFICATION.value
    assert "headers" not in result2
    assert "data" not in result2

    await registered_test_client.ack(result["channelID"], result["version"])
    await registered_test_client.ack(result2["channelID"], result2["version"])

    await registered_test_client.disconnect()
    await registered_test_client.send_notification(status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    result3 = await registered_test_client.get_notification()
    assert result3 is not None
    assert "headers" not in result3
    assert "data" not in result3
    await registered_test_client.ack(result3["channelID"], result3["version"])


async def test_big_message(registered_test_client: AsyncPushTestClient) -> None:
    """Test that we accept a large message.

    Using pywebpush I encoded a 4096 block
    of random data into a 4216b block. B64 encoding that produced a
    block that was 5624 bytes long. We'll skip the binary bit for a
    4216 block of "text" we then b64 encode to send.
    """
    import base64

    bulk = "".join(
        random.choice(string.ascii_letters + string.digits + string.punctuation)
        for _ in range(0, 4216)
    )
    data = base64.urlsafe_b64encode(bytes(bulk, "utf-8"))
    result = await registered_test_client.send_notification(data=data)
    dd = result.get("data")
    dh = base64.b64decode(dd + "==="[: len(dd) % 4])
    assert dh == data


# Need to dig into this test a bit more. I'm not sure it's structured
# correctly since we resolved a bug about returning 202 v. 201, and
# it's using a dependent library to do the Client calls. In short,
# this test will fail in `send_notification()` because the response
# will be a 202 instead of 201, and Client.send_notification will
# fail to record the message into it's internal message array, which will
# cause Client.delete_notification to fail.


# Skipping test for now.
# Note: dict_keys obj was not iterable, corrected by converting to iterable.
async def test_delete_saved_notification(registered_test_client: AsyncPushTestClient) -> None:
    """Test deleting a saved notification in client server."""
    await registered_test_client.disconnect()
    assert registered_test_client.channels
    chan = list(registered_test_client.channels.keys())[0]
    await registered_test_client.send_notification()
    status_code: int = 204
    delete_resp = await registered_test_client.delete_notification(chan, status=status_code)
    assert delete_resp.status_code == status_code
    await registered_test_client.connect()
    await registered_test_client.hello()
    result = await registered_test_client.get_notification()
    assert result is None


async def test_with_key(test_client: AsyncPushTestClient) -> None:
    """Test getting a locked subscription with a valid VAPID public key."""
    private_key = ecdsa.SigningKey.generate(curve=ecdsa.NIST256p)
    claims = {
        "aud": f"http://127.0.0.1:{ENDPOINT_PORT}",
        "exp": int(time.time()) + 86400,
        "sub": "a@example.com",
    }
    vapid = _get_vapid(private_key, claims)
    pk_hex = vapid["crypto-key"]
    chid = str(uuid.uuid4())
    await test_client.connect()
    await test_client.hello()
    await test_client.register(channel_id=chid, key=pk_hex)

    # Send an update with a properly formatted key.
    await test_client.send_notification(vapid=vapid)

    # now try an invalid key.
    new_key = ecdsa.SigningKey.generate(curve=ecdsa.NIST256p)
    vapid = _get_vapid(new_key, claims)

    await test_client.send_notification(vapid=vapid, status=401)


async def test_with_bad_key(test_client: AsyncPushTestClient):
    """Test that a message registration request with bad VAPID public key is rejected."""
    chid: str = str(uuid.uuid4())
    bad_key: str = "af1883%&!@#*("
    await test_client.connect()
    await test_client.hello()
    result = await test_client.register(channel_id=chid, key=bad_key, status=400)
    assert result["status"] == 400


@pytest.mark.parametrize("fixture_max_endpoint_logs", [44], indirect=True)
async def test_msg_limit(registered_test_client: AsyncPushTestClient):
    """Test that sent messages that are larger than our size limit are rejected."""
    uaid = registered_test_client.uaid
    await registered_test_client.disconnect()
    for i in range(MSG_LIMIT + 1):
        await registered_test_client.send_notification(status=201)
    await registered_test_client.connect()
    await registered_test_client.hello()
    assert registered_test_client.uaid == uaid
    for i in range(MSG_LIMIT):
        result = await registered_test_client.get_notification()
        assert result is not None, f"failed at {i}"
        await registered_test_client.ack(result["channelID"], result["version"])
    await registered_test_client.disconnect()
    await registered_test_client.connect()
    await registered_test_client.hello()
    assert registered_test_client.uaid != uaid


async def test_can_moz_ping(registered_test_client) -> None:
    """Test that the client can send a small ping message and get a valid response."""
    result = await registered_test_client.moz_ping()
    assert result == "{}"
    assert registered_test_client.ws.open
    try:
        await registered_test_client.moz_ping()
    except websockets.exceptions.ConnectionClosedError:
        # pinging too quickly should disconnect without a valid ping
        # repsonse
        pass
    assert not registered_test_client.ws.open


async def test_internal_endpoints(
    ws_url: str, registered_test_client: AsyncPushTestClient
) -> None:
    """Ensure an internal router endpoint isn't exposed on the public CONNECTION_PORT"""
    parsed = (
        urlparse(ws_url)
        ._replace(scheme="http")
        ._replace(path=f"/notif/{registered_test_client.uaid}")
    )

    # We can't determine an AUTOPUSH_CN_SERVER's ROUTER_PORT
    if not os.getenv("AUTOPUSH_CN_SERVER"):
        url = parsed._replace(netloc=f"{parsed.hostname}:{ROUTER_PORT}").geturl()
        # First ensure the endpoint we're testing for on the public port exists where
        # we expect it on the internal ROUTER_PORT
        async with httpx.AsyncClient() as httpx_client:
            res = await httpx_client.put(url, timeout=30)
            res.raise_for_status()

    try:
        async with httpx.AsyncClient() as httpx_client:
            res = await httpx_client.put(parsed.geturl(), timeout=30)
            res.raise_for_status()
    except httpx.ConnectError:
        pass
    except httpx.HTTPError as e:
        assert e.response.status_code == 404
    else:
        assert False


@pytest.mark.parametrize(
    "fixture_max_endpoint_logs, fixture_max_conn_logs",
    [[4, 1]],
    indirect=["fixture_max_endpoint_logs", "fixture_max_conn_logs"],
)
async def test_broadcast_update_on_connect(test_client_broadcast: AsyncPushTestClient) -> None:
    """Test that the client receives any pending broadcast updates on connect."""
    global MOCK_MP_SERVICES
    MOCK_MP_SERVICES = {"kinto:123": "ver1"}
    MOCK_MP_POLLED.clear()
    MOCK_MP_POLLED.wait(timeout=5)
    old_ver = {"kinto:123": "ver0"}
    await test_client_broadcast.connect()
    result = await test_client_broadcast.hello(services=old_ver)
    assert result != {}
    assert result["use_webpush"] is True
    assert result["broadcasts"]["kinto:123"] == "ver1"

    MOCK_MP_SERVICES = {"kinto:123": "ver2"}
    MOCK_MP_POLLED.clear()
    MOCK_MP_POLLED.wait(timeout=5)

    result = await test_client_broadcast.get_broadcast(2)
    assert result.get("messageType") == ClientMessageType.BROADCAST.value
    assert result["broadcasts"]["kinto:123"] == "ver2"


@pytest.mark.parametrize(
    "fixture_max_endpoint_logs, fixture_max_conn_logs",
    [[4, 1]],
    indirect=["fixture_max_endpoint_logs", "fixture_max_conn_logs"],
)
async def test_broadcast_update_on_connect_with_errors(
    test_client_broadcast: AsyncPushTestClient,
) -> None:
    """Test that the client can receive broadcast updates on connect
    that may have produced internal errors.
    """
    global MOCK_MP_SERVICES
    MOCK_MP_SERVICES = {"kinto:123": "ver1"}
    MOCK_MP_POLLED.clear()
    MOCK_MP_POLLED.wait(timeout=5)

    old_ver = {"kinto:123": "ver0", "kinto:456": "ver1"}
    await test_client_broadcast.connect()
    result = await test_client_broadcast.hello(services=old_ver)
    assert result != {}
    assert result["use_webpush"] is True
    assert result["broadcasts"]["kinto:123"] == "ver1"
    assert result["broadcasts"]["errors"]["kinto:456"] == "Broadcast not found"


@pytest.mark.parametrize(
    "fixture_max_endpoint_logs, fixture_max_conn_logs",
    [[4, 1]],
    indirect=["fixture_max_endpoint_logs", "fixture_max_conn_logs"],
)
async def test_broadcast_subscribe(test_client_broadcast: AsyncPushTestClient) -> None:
    """Test that the client can subscribe to new broadcasts."""
    global MOCK_MP_SERVICES
    MOCK_MP_SERVICES = {"kinto:123": "ver1"}
    MOCK_MP_POLLED.clear()
    MOCK_MP_POLLED.wait(timeout=5)

    old_ver = {"kinto:123": "ver0"}
    await test_client_broadcast.connect()
    result = await test_client_broadcast.hello()
    assert result != {}
    assert result["use_webpush"] is True
    assert result["broadcasts"] == {}

    await test_client_broadcast.broadcast_subscribe(old_ver)
    result = await test_client_broadcast.get_broadcast()
    assert result.get("messageType") == ClientMessageType.BROADCAST.value
    assert result["broadcasts"]["kinto:123"] == "ver1"

    MOCK_MP_SERVICES = {"kinto:123": "ver2"}
    MOCK_MP_POLLED.clear()
    MOCK_MP_POLLED.wait(timeout=5)

    result = await test_client_broadcast.get_broadcast(2)
    assert result.get("messageType") == ClientMessageType.BROADCAST.value
    assert result["broadcasts"]["kinto:123"] == "ver2"


@pytest.mark.parametrize(
    "fixture_max_endpoint_logs, fixture_max_conn_logs",
    [[4, 1]],
    indirect=["fixture_max_endpoint_logs", "fixture_max_conn_logs"],
)
async def test_broadcast_subscribe_with_errors(test_client_broadcast: AsyncPushTestClient) -> None:
    """Test that broadcast returns expected errors."""
    global MOCK_MP_SERVICES
    MOCK_MP_SERVICES = {"kinto:123": "ver1"}
    MOCK_MP_POLLED.clear()
    MOCK_MP_POLLED.wait(timeout=5)

    old_ver = {"kinto:123": "ver0", "kinto:456": "ver1"}
    await test_client_broadcast.connect()
    result = await test_client_broadcast.hello()
    assert result != {}
    assert result["use_webpush"] is True
    assert result["broadcasts"] == {}

    await test_client_broadcast.broadcast_subscribe(old_ver)
    result = await test_client_broadcast.get_broadcast()
    assert result.get("messageType") == ClientMessageType.BROADCAST.value
    assert result["broadcasts"]["kinto:123"] == "ver1"
    assert result["broadcasts"]["errors"]["kinto:456"] == "Broadcast not found"


@pytest.mark.parametrize(
    "fixture_max_endpoint_logs, fixture_max_conn_logs",
    [[4, 1]],
    indirect=["fixture_max_endpoint_logs", "fixture_max_conn_logs"],
)
async def test_broadcast_no_changes(test_client_broadcast: AsyncPushTestClient) -> None:
    """Test to ensure there are no changes from broadcast."""
    global MOCK_MP_SERVICES
    MOCK_MP_SERVICES = {"kinto:123": "ver1"}
    MOCK_MP_POLLED.clear()
    MOCK_MP_POLLED.wait(timeout=5)

    old_ver = {"kinto:123": "ver1"}
    await test_client_broadcast.connect()
    result = await test_client_broadcast.hello(services=old_ver)
    assert result != {}
    assert result["use_webpush"] is True
    assert result["broadcasts"] == {}


# Stub Tests
# these tests are only really useful for testing the stub system and are
# not required for production CI testing.
async def test_mobile_register_v1(test_client: AsyncPushTestClient) -> None:
    """Test that the mobile "hello" request returns an unsigned endpoint
    if no `key` is included in the body
    """
    if not os.getenv("TEST_STUB"):
        pytest.skip("Skipping stub test test_mobile_register_v1")

    endpoint = test_client.get_host_client_endpoint()
    async with httpx.AsyncClient() as httpx_client:
        resp = await httpx_client.request(
            method="POST",
            url=f"{endpoint}/v1/stub/success/registration",
            headers={"content-type": "application/json"},
            content=json.dumps({"token": "success"}),
        )  # nosec
        assert resp.status_code == 200, "Could not register stub endpoint"
        response = resp.json()
        endpoint = response.get("endpoint")
        assert endpoint is not None
        assert urlparse(endpoint).path.split("/")[2] == "v1"
        assert response.get("secret") is not None
        assert response.get("uaid") is not None


async def test_mobile_register_v2(test_client: AsyncPushTestClient) -> None:
    """Test that a signed endpoint is returned if a valid VAPID public
    key is included in the body.
    """
    if not os.getenv("TEST_STUB"):
        pytest.skip("Skipping stub test test_mobile_register_v2")
    vapid_pub = (
        "BBO5r087l4d3kxx9INyRenewaA5WOWiaSFqy77UXN7ZRVxr3gNtyWeP"
        "CjUbOerY1xUUcUFCtVoT5vdElIxTLlCc"
    )
    endpoint = test_client.get_host_client_endpoint()
    async with httpx.AsyncClient() as httpx_client:
        resp = await httpx_client.request(
            method="POST",
            url=f"{endpoint}/v1/stub/success/registration",
            headers={"content-type": "application/json"},
            content=json.dumps({"token": "success", "key": vapid_pub}),
        )  # nosec
        assert resp.status_code == 200
        response = resp.json()
        endpoint = response.get("endpoint")
        secret = response.get("secret")
        uaid = response.get("uaid")
        assert response.get("channelID") is not None
        assert endpoint is not None
        assert urlparse(endpoint).path.split("/")[2] == "v2"
        assert secret is not None
        assert uaid is not None

    # and check to see that we can subscribe to a new channel and pass
    # in an explicit channelID.
    chid2 = str(uuid.uuid4())
    async with httpx.AsyncClient() as httpx_client:
        resp = await httpx_client.request(
            method="POST",
            url=f"{endpoint}/v1/stub/success/registration/{uaid}/subscription",
            headers={"content-type": "application/json", "authorization": f"webpush {secret}"},
            content=json.dumps({"token": "success", "channelID": chid2, "key": vapid_pub}),
        )  # nosec
        assert resp.status_code == 200
        resp2 = resp.json()
        end2 = resp2.get("endpoint")
        assert end2 is not None and end2 != endpoint
        assert chid2 == resp2.get("channelID")
