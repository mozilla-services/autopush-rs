"""Configuration module for autopush-rs integration tests."""
import copy
import json
import os
import socket
import subprocess
import uuid
from pathlib import Path
from queue import Queue
from threading import Event, Thread
from typing import Any

import twisted
from cryptography.fernet import Fernet

integration_dir: Path = Path(".").absolute()
tests_dir: Path = integration_dir.parent
root_dir: Path = tests_dir.parent

DDB_JAR: Path = integration_dir / "ddb" / "DynamoDBLocal.jar"
DDB_LIB_DIR: Path = integration_dir / "ddb" / "DynamoDBLocal_lib"
SETUP_BT_SH: Path = root_dir / "scripts" / "setup_bt.sh"
DDB_PROCESS: subprocess.Popen | None = None
BT_PROCESS: subprocess.Popen | None = None
BT_DB_SETTINGS: str | None = None

twisted.internet.base.DelayedCall.debug = True

ROUTER_TABLE = os.getenv("ROUTER_TABLE", "router_int_test")
MESSAGE_TABLE = os.getenv("MESSAGE_TABLE", "message_int_test")
MSG_LIMIT: int = 20

CRYPTO_KEY = os.getenv("CRYPTO_KEY") or Fernet.generate_key().decode("utf-8")
CONNECTION_PORT: int = 9150
ENDPOINT_PORT: int = 9160
ROUTER_PORT: int = 9170
MP_CONNECTION_PORT: int = 9052
MP_ROUTER_PORT: int = 9072

CONNECTION_BINARY = os.getenv("CONNECTION_BINARY", "autoconnect")
CONNECTION_SETTINGS_PREFIX = os.getenv("CONNECTION_SETTINGS_PREFIX", "autoconnect__")

CN_SERVER: subprocess.Popen | None = None
CN_MP_SERVER: subprocess.Popen | None = None
EP_SERVER: subprocess.Popen | None = None
MOCK_SERVER_THREAD: Thread | None = None
CN_QUEUES: list = []
EP_QUEUES: list = []
STRICT_LOG_COUNTS: bool = True

modules: list[str] = [
    "autoconnect",
    "autoconnect_common",
    "autoconnect_web",
    "autoconnect_ws",
    "autoconnect_ws_sm",
    "autoendpoint",
    "autopush",
    "autopush_common",
]

log_string: list[str] = [f"{x}=trace" for x in modules]
RUST_LOG: str = f"{','.join(log_string)},error"


def get_free_port() -> int:
    """Get free websocket port."""
    port: int
    s = socket.socket(socket.AF_INET, type=socket.SOCK_STREAM)
    s.bind(("localhost", 0))
    address, port = s.getsockname()
    s.close()
    return port


def get_db_settings() -> str | dict[str, str | int | float] | None:
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
        file_path = integration_dir / env_var
        if file_path.is_file():
            with file_path.open(mode="r") as file:
                return file.read()
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
