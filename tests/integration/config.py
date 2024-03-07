"""Configuration module for autopush-rs integration tests."""
import copy
import json
import logging
import os
import socket
import subprocess
import time
import uuid
from dataclasses import asdict, dataclass
from pathlib import Path
from queue import Queue
from threading import Event, Thread
from typing import Any
from urllib.parse import urlparse

import ecdsa
from cryptography.fernet import Fernet
from jose import jws

from .db import (
    DynamoDBResource,
    base64url_encode,
    create_message_table_ddb,
    get_router_table,
)

logging.basicConfig(level=logging.DEBUG)
log = logging.getLogger(__name__)

integration_dir: Path = Path(__file__).resolve().parent
tests_dir: Path = integration_dir.parent
root_dir: Path = tests_dir.parent
INTEGRATION_DIR: Path = Path(__file__).resolve().parent
TESTS_DIR: Path = INTEGRATION_DIR.parent
ROOT_DIR: Path = TESTS_DIR.parent

ROUTER_TABLE = os.environ.get("ROUTER_TABLE", "router_int_test")
MESSAGE_TABLE = os.environ.get("MESSAGE_TABLE", "message_int_test")
MSG_LIMIT = 20

CRYPTO_KEY = os.environ.get("CRYPTO_KEY") or Fernet.generate_key().decode("utf-8")
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
    "autopush",
    "autopush_common",
]
log_string = [f"{x}=trace" for x in modules]
RUST_LOG = ",".join(log_string) + ",error"


class Config:
    """Configuration class containing shared configuration variables for test suite."""

    INTEGRATION_DIR: Path = Path(__file__).resolve().parent
    TESTS_DIR: Path = INTEGRATION_DIR.parent
    ROOT_DIR: Path = TESTS_DIR.parent

    ROUTER_TABLE = os.environ.get("ROUTER_TABLE", "router_int_test")
    MESSAGE_TABLE = os.environ.get("MESSAGE_TABLE", "message_int_test")
    MSG_LIMIT = 20

    CRYPTO_KEY = os.environ.get("CRYPTO_KEY") or Fernet.generate_key().decode("utf-8")
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
        "autopush",
        "autopush_common",
    ]
    log_string = [f"{x}=trace" for x in modules]
    RUST_LOG = ",".join(log_string) + ",error"

    DDB_PROCESS: subprocess.Popen | None = None
    BT_PROCESS: subprocess.Popen | None = None
    BT_DB_SETTINGS: str | None = None

    MOCK_SERVER_PORT: Any = None
    MOCK_MP_SERVICES: dict = {}
    MOCK_MP_TOKEN: str = f"Bearer {uuid.uuid4().hex}"
    MOCK_MP_POLLED: Event = Event()
    MOCK_SENTRY_QUEUE: Queue = Queue()

    def __init__(self):
        self.MOCK_SERVER_PORT = self.get_free_port()

    @staticmethod
    def get_free_port() -> int:
        """Get free websocket port."""
        port: int
        s = socket.socket(socket.AF_INET, type=socket.SOCK_STREAM)
        s.bind(("localhost", 0))
        address, port = s.getsockname()
        s.close()
        return port

    @staticmethod
    def write_config_to_env(config: dict[str, Any], prefix: str):
        """Write configurations to application read environment variables."""
        for key, val in config.items():
            new_key = prefix + key
            log.debug(f"✍ config {new_key} => {val}")
            os.environ[new_key.upper()] = str(val)


MOCK_SERVER_PORT: Any = Config.get_free_port()
MOCK_MP_SERVICES: dict = {}
MOCK_MP_TOKEN: str = f"Bearer {uuid.uuid4().hex}"
MOCK_MP_POLLED: Event = Event()
MOCK_SENTRY_QUEUE: Queue = Queue()


@dataclass
class DatabaseConfig:
    """Core configuration and setup class for databases in Push Integration Tests."""

    DDB_JAR: Path = Config.INTEGRATION_DIR / "ddb" / "DynamoDBLocal.jar"
    DDB_LIB_DIR: Path = Config.INTEGRATION_DIR / "ddb" / "DynamoDBLocal_lib"
    SETUP_BT_SH: Path = Config.ROOT_DIR / "scripts" / "setup_bt.sh"
    DDB_PROCESS: subprocess.Popen | None = None
    BT_PROCESS: subprocess.Popen | None = None
    BT_DB_SETTINGS: str | None = None

    @classmethod
    def get_db_settings(cls) -> str | dict[str, str | int | float] | None:
        """Try reading the database settings (DB_SETTINGS) from the environment.

        Returns
        -------
        str | dict
            If DB_SETTINGS points to a file, read the settings from that file.
            If DB_SETTINGS is not a file, return string env var.
            Otherwise, return default defined

        """
        env_var = os.environ.get("DB_SETTINGS")
        if env_var:
            file_path = Config.INTEGRATION_DIR / env_var
            if file_path.is_file():
                with file_path.open(mode="r") as file:
                    return file.read()
            return env_var

        return json.dumps(
            dict(
                router_table=Config.ROUTER_TABLE,
                message_table=Config.MESSAGE_TABLE,
                current_message_month=Config.MESSAGE_TABLE,
                table_name="projects/test/instances/test/tables/autopush",
                router_family="router",
                message_family="message",
                message_topic_family="message_topic",
            )
        )

    def setup_bt(self):
        """Set up BigTable emulator."""
        log.debug("🐍🟢 Starting bigtable emulator")
        self.BT_PROCESS = subprocess.Popen(
            "gcloud beta emulators bigtable start".split(" ")
        )  # nosec
        os.environ["BIGTABLE_EMULATOR_HOST"] = "localhost:8086"
        try:
            self.BT_DB_SETTINGS = os.environ.get(
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
            vv = subprocess.call([self.SETUP_BT_SH])  # nosec
            log.debug(vv)
            return self.BT_PROCESS
        except Exception as e:
            log.error("Bigtable Setup Error {}", e)
            raise

    def setup_dynamodb(self):
        """Set up DynamoDB emulator."""
        log.debug("🐍🟢 Starting dynamodb")
        if os.getenv("AWS_LOCAL_DYNAMODB") is None:
            cmd = " ".join(
                [
                    "java",
                    f"-Djava.library.path={self.DDB_LIB_DIR}",
                    "-jar",
                    f"{self.DDB_JAR}",
                    "-sharedDb",
                    "-inMemory",
                ]
            )
            self.DDB_PROCESS = subprocess.Popen(cmd, shell=True, env=os.environ)  # nosec
            os.environ["AWS_LOCAL_DYNAMODB"] = "http://127.0.0.1:8000"
            return self.DDB_PROCESS
        else:
            print("Using existing DynamoDB instance")

        # Setup the necessary tables
        boto_resource = DynamoDBResource()
        create_message_table_ddb(boto_resource, Config.MESSAGE_TABLE)
        get_router_table(boto_resource, Config.ROUTER_TABLE)


"""Connection Server Config:
For local test debugging, set `AUTOPUSH_CN_CONFIG=_url_` to override
creation of the local server.
"""
CONNECTION_CONFIG: dict[str, Any] = dict(
    hostname="localhost",
    port=CONNECTION_PORT,
    endpoint_hostname="localhost",
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
    db_dsn=os.getenv("DB_DSN", "http://127.0.0.1:8000"),
    db_settings=DatabaseConfig.get_db_settings(),
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
    megaphone_api_url=f"http://localhost:{MOCK_SERVER_PORT}/v1/broadcasts",
    megaphone_api_token=MOCK_MP_TOKEN,
    megaphone_poll_interval=1,
)

"""Endpoint Server Config:
For local test debugging, set `AUTOPUSH_EP_CONFIG=_url_` to override
creation of the local server.
"""
ENDPOINT_CONFIG = dict(
    host="localhost",
    port=ENDPOINT_PORT,
    router_table_name=ROUTER_TABLE,
    message_table_name=MESSAGE_TABLE,
    human_logs="true",
    crypto_keys=f"[{CRYPTO_KEY}]",
)


def _get_vapid(
    key: ecdsa.SigningKey | None = None,
    payload: dict[str, str | int] | None = None,
    endpoint: str | None = None,
) -> dict[str, str | bytes]:
    """Get VAPID information, including the `Authorization` header string,
    public and private keys.
    """
    global CONNECTION_CONFIG

    if endpoint is None:
        endpoint = "{}://{}:{}".format(
            CONNECTION_CONFIG.get("endpoint_scheme"),
            CONNECTION_CONFIG.get("endpoint_hostname"),
            CONNECTION_CONFIG.get("endpoint_port"),
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
    vk: ecdsa.VerifyingKey = key.get_verifying_key()
    auth: str = jws.sign(payload, key, algorithm="ES256").strip("=")
    crypto_key: str = base64url_encode((b"\4" + vk.to_string()))
    return {"auth": auth, "crypto-key": crypto_key, "key": key}


def enqueue_output(out, queue):
    """Add lines from the out buffer to the provided queue."""
    for line in iter(out.readline, ""):
        queue.put(line)
    out.close()


def capture_output_to_queue(output_stream):
    """Capture output to log queue."""
    log_queue = Queue()
    t = Thread(target=enqueue_output, args=(output_stream, log_queue))
    t.daemon = True  # thread dies with the program
    t.start()
    return log_queue


def get_rust_binary_path(binary):
    """Get path to pre-built Rust binary.
    This presumes that the application has already been built with proper features.
    """
    global STRICT_LOG_COUNTS

    rust_bin = str(root_dir) + "/target/release/{}".format(binary)
    possible_paths = [
        "/target/debug/{}".format(binary),
        "/{0}/target/release/{0}".format(binary),
        "/{0}/target/debug/{0}".format(binary),
    ]
    while possible_paths and not os.path.exists(rust_bin):  # pragma: nocover
        rust_bin = str(root_dir) + possible_paths.pop(0)

    if "release" not in rust_bin:
        # disable checks for chatty debug mode binaries
        STRICT_LOG_COUNTS = False

    return rust_bin


@dataclass
class ConnectionServer:
    """Connection Server Config:
    For local test debugging, set `AUTOPUSH_CN_CONFIG=_url_` to override
    creation of the local server.
    """

    hostname: str = "localhost"
    port: int = Config.CONNECTION_PORT
    endpoint_hostname: str = "localhost"
    endpoint_port: int = Config.ENDPOINT_PORT
    router_port: int = Config.ROUTER_PORT
    endpoint_scheme: str = "http"
    router_tablename: str = Config.ROUTER_TABLE
    message_tablename: str = Config.MESSAGE_TABLE
    crypto_key: str = f"[{Config.CRYPTO_KEY}]"
    auto_ping_interval: float = 30.0
    auto_ping_timeout: float = 10.0
    close_handshake_timeout: int = 5
    max_connections: int = 5000
    human_logs: str = "true"
    msg_limit: int = Config.MSG_LIMIT
    # new autoconnect
    db_dsn: str = os.getenv("DB_DSN", "http://127.0.0.1:8000")
    db_settings: Any = DatabaseConfig.get_db_settings()

    def to_dict(self):
        """Convert attributes of ConnectionServer class to dictionary."""
        return {k: str(v) for k, v in asdict(self).items()}

    def setup_connection_server(self, connection_binary):
        """Set up connection server from config."""
        # NOTE: due to a change in Config, autopush uses a double
        # underscore as a separator (e.g. "AUTOEND__FCM__MIN_TTL" ==
        # `settings.fcm.min_ttl`)

        url: str | None = os.getenv("AUTOPUSH_CN_SERVER")
        if url is not None:
            parsed = urlparse(url)
            self.hostname = parsed.hostname
            self.port = parsed.port
            self.endpoint_scheme = parsed.scheme
            Config.write_config_to_env(self.to_dict(), Config.CONNECTION_SETTINGS_PREFIX)
            log.debug("Using existing Connection server")
            return
        else:
            Config.write_config_to_env(self.to_dict(), Config.CONNECTION_SETTINGS_PREFIX)
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


@dataclass
class MegaphoneServer(ConnectionServer):
    """Connection Megaphone Config:
    For local test debugging, set `AUTOPUSH_MP_CONFIG=_url_` to override
    creation of the local server.
    """

    port: int = Config.MP_CONNECTION_PORT
    router_port: int = Config.MP_ROUTER_PORT
    endpoint_scheme: str = "http"
    router_tablename: str = Config.ROUTER_TABLE
    auto_ping_interval: float = 0.5
    megaphone_api_url: str = f"http://localhost:{Config.MOCK_SERVER_PORT}/v1/broadcasts"
    megaphone_api_token: str = Config.MOCK_MP_TOKEN
    megaphone_poll_interval: int = 1

    def setup_megaphone_server(self, connection_binary):
        """Set up megaphone server from configuration."""
        global CN_MP_SERVER
        url: str | None = os.getenv("AUTOPUSH_MP_SERVER")
        if url is not None:
            parsed = urlparse(url)
            self.hostname = parsed.hostname
            self.port = parsed.port
            self.endpoint_scheme = parsed.scheme
            url = os.getenv("AUTOPUSH_EP_SERVER")
            if url is not None:
                parsed = urlparse(url)
                self.endpoint_port = parsed.port
            Config.write_config_to_env(self.to_dict(), CONNECTION_SETTINGS_PREFIX)
            log.debug("Using existing Megaphone server")
            return
        else:
            Config.write_config_to_env(self.to_dict(), CONNECTION_SETTINGS_PREFIX)
        cmd = [connection_binary]
        log.debug("🐍🟢 Starting Megaphone server: {}".format(" ".join(cmd)))
        CN_MP_SERVER = subprocess.Popen(cmd, shell=True, env=os.environ)  # nosec


@dataclass
class EndpointServer:
    """Endpoint Server Config:
    For local test debugging, set `AUTOPUSH_EP_CONFIG=_url_` to override
    creation of the local server.
    """

    host: str = "localhost"
    port: int = ENDPOINT_PORT
    router_table_name: str = ROUTER_TABLE
    message_table_name: str = MESSAGE_TABLE
    human_logs: str = "true"
    crypto_keys: str = f"[{CRYPTO_KEY}]"
    db_dsn: str = os.getenv("DB_DSN", "http://127.0.0.1:8000")
    db_settings: Any = DatabaseConfig.get_db_settings()

    def setup_endpoint_server(self):
        """Set up endpoint server from configuration."""
        global CONNECTION_CONFIG, EP_SERVER, BT_PROCESS

        # Set up environment
        # NOTE: due to a change in Config, autoendpoint uses a double
        # underscore as a separator (e.g. "AUTOEND__FCM__MIN_TTL" ==
        # `settings.fcm.min_ttl`)
        url = os.getenv("AUTOPUSH_EP_SERVER")
        # self.db_dsn = CONNECTION_CONFIG["db_dsn"]
        # self.db_settings = CONNECTION_CONFIG["db_settings"]
        if url is not None:
            parsed = urlparse(url)
            self.hostname = parsed.hostname
            self.port = parsed.port
            self.endpoint_scheme = parsed.scheme
            log.debug("Using existing Endpoint server")
            return
        else:
            Config.write_config_to_env(ENDPOINT_CONFIG, "autoend__")

        # Run autoendpoint
        cmd = [get_rust_binary_path("autoendpoint")]

        log.debug("🐍🟢 Starting Endpoint server: {}".format(" ".join(cmd)))
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
