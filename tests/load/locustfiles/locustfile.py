"""Performance test module."""

# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

import base64
import json
import logging
import os
import random
import string
import time
import uuid
from hashlib import sha256
from json import JSONDecodeError
from logging import Logger
from typing import Any, TypeAlias, cast
from urllib.parse import urlparse

import gevent
import websocket
from args import parse_wait_time
from exceptions import ZeroStatusRequestError
from gevent import Greenlet
from locust import FastHttpUser, events, task
from locust.exception import LocustError
from models import (
    HelloMessage,
    HelloRecord,
    NotificationMessage,
    NotificationRecord,
    RegisterMessage,
    RegisterRecord,
    UnregisterMessage,
)
from py_vapid import Vapid02
from pydantic import ValidationError
from websocket import WebSocket, WebSocketApp, WebSocketConnectionClosedException

Message: TypeAlias = HelloMessage | NotificationMessage | RegisterMessage | UnregisterMessage
Record: TypeAlias = HelloRecord | NotificationRecord | RegisterRecord

# Set to 'True' to view the verbose connection information for the web socket
websocket.enableTrace(False)
websocket.setdefaulttimeout(5)

logger: Logger = logging.getLogger("AutopushUser")


@events.init_command_line_parser.add_listener
def _(parser: Any):
    parser.add_argument(
        "--wait_time",
        type=str,
        env_var="AUTOPUSH_WAIT_TIME",
        help="AutopushUser wait time between tasks",
        default="25, 30",
    )
    parser.add_argument(
        "--vapid_key",
        type=str,
        env_var="AUTOPUSH_VAPID_KEY",
        help="Path to an optional VAPID private key.",
    )


@events.test_start.add_listener
def _(environment, **kwargs):
    environment.autopush_wait_time = parse_wait_time(environment.parsed_options.wait_time)
    environment.vapid = None
    if environment.parsed_options.vapid_key:
        try:
            if os.path.isfile(environment.parsed_options.vapid_key):
                logging.info(f"🔍 Vapid key requested. {environment.parsed_options.vapid_key=}")
                environment.vapid = Vapid02.from_file(environment.parsed_options.vapid_key)
            else:
                logging.error(
                    f"🔍 VAPID key file not found: {environment.parsed_options.vapid_key}"
                )
        except ValueError as error:
            raise LocustError(
                f"🔍 Invalid VAPID key provided: {error}. "
                "Please provide a valid VAPID private key path."
            ) from error


class AutopushUser(FastHttpUser):
    """AutopushUser class."""

    REST_HEADERS: dict[str, str] = {"TTL": "60", "Content-Encoding": "aes128gcm"}
    WEBSOCKET_HEADERS: dict[str, str] = {
        "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.13; rv:61.0) "
        "Gecko/20100101 Firefox/61.0"
    }

    def __init__(self, environment) -> None:
        super().__init__(environment)
        self.channels: dict[str, str] = {}
        self.hello_record: HelloRecord | None = None
        self.notification_records: dict[str, NotificationRecord] = {}
        self.purged_records: set[str] = set()
        self.register_records: dict[str, RegisterRecord] = {}
        self.unregister_records: dict[str, RegisterRecord] = {}
        self.uaid: str = ""
        self.ws: WebSocketApp | None = None
        self.ws_greenlet: Greenlet | None = None
        try:
            vapid = environment.vapid
        except AttributeError:
            vapid = None
        self.vapid: Vapid02 | None = vapid

    def gen_message_key(self, data: str) -> str:
        """Generate a unique key for a message based on its data."""
        digest = sha256(data.encode(), usedforsecurity=False).digest()  # nosec
        return hex(int.from_bytes(digest, "big"))

    def wait_time(self):
        """Return the autopush wait time."""
        return self.environment.autopush_wait_time(self)

    def on_start(self) -> Any:
        """Call when a User starts running."""
        self.ws_greenlet = gevent.spawn(self.connect)

    def on_stop(self) -> Any:
        """Call when a User stops running."""
        if self.ws:
            for channel_id in self.channels.keys():
                self.send_unregister(self.ws, channel_id)
            self.ws.close()
            self.ws = None
        if self.ws_greenlet:
            gevent.kill(self.ws_greenlet)

    def on_ws_open(self, ws: WebSocketApp) -> None:
        """Call when opening a WebSocket.

        Parameters
        ----------
        ws : WebSocket
        """
        if ws.sock:
            self.send_hello(ws.sock)

    def on_ws_message(self, ws: WebSocketApp, data: str) -> None:
        """Call when received data from a WebSocket.

        Parameters
        ----------
        ws : WebSocket
        data : str
            utf-8 data received from the server
        """
        message: Message | None = self.recv(data)
        if isinstance(message, HelloMessage):
            self.uaid = message.uaid
        elif isinstance(message, NotificationMessage) and ws.sock:
            key = self.gen_message_key(base64.urlsafe_b64decode(message.data + "====").decode())
            logger.info(f"Acking message {key} :: {message.version}")
            self.send_ack(ws.sock, message.channelID, message.version)
        elif isinstance(message, RegisterMessage):
            self.channels[message.channelID] = message.pushEndpoint
        elif isinstance(message, UnregisterMessage):
            del self.channels[message.channelID]

    def on_ws_error(self, ws: WebSocketApp, error: Exception) -> None:
        """Call when there is a WebSocket error or if an exception is raised in a WebSocket
        callback function.

        Parameters
        ----------
        ws : WebSocket
        error : Exception
        """
        logger.error(str(error))

        # WebSocket closures are expected
        if isinstance(error, WebSocketConnectionClosedException):
            return  # Don't send the exception to the UI it creates too much clutter

        self.environment.events.user_error.fire(
            user_instance=self.context(), exception=error, tb=error.__traceback__
        )

    def on_ws_close(
        self, ws: WebSocketApp, close_status_code: int | None, close_msg: str | None
    ) -> None:
        """Call when closing a WebSocket.

        Parameters
        ----------
        ws: WebSocket
        close_status_code : int | None
            WebSocket close status
        close_msg : str | None
            ebSocket close message
        """
        if close_status_code or close_msg:
            logger.info(f"WebSocket closed. status={close_status_code} msg={close_msg}")

    @task(weight=98)
    def send_notification(self) -> None:
        """Send a notification to a registered endpoint while connected to Autopush."""
        if not self.ws or not self.channels:
            logger.debug("Task 'send_notification' skipped.")
            return

        endpoint_url: str = random.choice(list(self.channels.values()))  # nosec
        self.post_notification(endpoint_url)

    @task(weight=1)
    def subscribe(self):
        """Subscribe a user to an Autopush channel."""
        if not self.ws:
            logger.debug("Task 'subscribe' skipped.")
            return

        channel_id: str = str(uuid.uuid4())
        self.send_register(self.ws, channel_id)

    @task(weight=1)
    def unsubscribe(self) -> None:
        """Unsubscribe a user from an Autopush channel."""
        if not self.ws or not self.channels:
            logger.debug("Task 'unsubscribe' skipped.")
            return

        channel_id: str = random.choice(list(self.channels.keys()))  # nosec
        self.send_unregister(self.ws, channel_id)

    def connect(self) -> None:
        """Create the WebSocketApp that will run indefinitely."""
        if not self.host:
            raise LocustError("'host' value is unavailable.")

        self.ws = websocket.WebSocketApp(
            self.host,
            header=self.WEBSOCKET_HEADERS,
            on_message=self.on_ws_message,
            on_error=self.on_ws_error,
            on_close=self.on_ws_close,
            on_open=self.on_ws_open,
        )

        # If reconnect is set to 0 (default) the WebSocket will not reconnect.
        self.ws.run_forever(reconnect=1)

    def post_notification(self, endpoint_url: str) -> None:
        """Send a notification to Autopush.

        Parameters
        ----------
        endpoint_url : str
            A channel destination endpoint url

        Raises
        ------
        ZeroStatusRequestError
            In the event that Locust experiences a network issue while
            sending a notification.
        """
        message_type: str = "notification"
        # Prefix random message with 'TestData' to more easily differentiate the payload
        data: str = "TestData" + "".join(
            [
                random.choice(string.ascii_letters + string.digits)  # nosec
                for i in range(0, random.randrange(1024, 4096, 2) - 8)  # nosec
            ]
        )

        record = NotificationRecord(send_time=time.perf_counter(), data=data)
        headers = self.REST_HEADERS
        if self.vapid:
            logging.info("🔍 Using VAPID key for Autopush notification.")
            parsed = urlparse(endpoint_url)
            host = f"{parsed.scheme}://{parsed.netloc}"
            # The key should already be created.
            vapid = self.vapid.sign(
                claims={
                    "sub": "mailto:loadtest@example.com",
                    "aud": host,
                    "exp": int(time.time()) + 86400,
                }
            )
            headers = self.REST_HEADERS.copy()
            headers.update(vapid)
        key = self.gen_message_key(data)
        logging.info(f"storing: {key}")
        self.notification_records[key] = record  # nosec

        with self.client.post(
            url=endpoint_url,
            name=message_type,
            data=data,
            headers=headers,
            catch_response=True,
        ) as response:
            if response.status_code == 0:
                raise ZeroStatusRequestError()
            if response.status_code != 201:
                response.failure(f"{response.status_code=}, expected 201, {response.text=}")

    def recv(self, data: str) -> Message | None:
        """Verify the contents of an Autopush message and report response statistics to Locust.

        Parameters
        ----------
        data : str
            utf-8 data received from the server

        Returns
        -------
        Message | None
            TypeAlias for multiple Message children
            HelloMessage | NotificationMessage | RegisterMessage | UnregisterMessage

        Raises
        ------
        ValidationError | JSONDecodeError
        """
        recv_time: float = time.perf_counter()
        exception: str | None = None
        message: Message | None = None
        message_type: str = "unknown"
        record: Record | None = None
        response_time: float = 0

        try:
            message_dict: dict[str, Any] = json.loads(data)
            message_type = message_dict.get("messageType", "unknown")
            key = None
            match message_type:
                case "hello":
                    message = HelloMessage(**message_dict)
                    record = self.hello_record
                case "notification":
                    message = NotificationMessage(**message_dict)
                    message_data: str = message.data
                    # scan through the notification records to see
                    # if this matches a record we sent.
                    # (Remember, we get back a stripped, base64 encoded version of the data)
                    decoded = base64.urlsafe_b64decode(message_data.encode() + b"====")
                    key = self.gen_message_key(decoded.decode())
                    logging.info(f"looking for: {key}")
                    record = self.notification_records.get(key, None)  # nosec
                    if not record:
                        logger.error(
                            f"No record found for {key}. Contents: {decoded[:100].decode()}..."
                        )
                    else:
                        self.purged_records.add(key)
                        logger.info(f"removing {key}")
                    # TODO: We should ACK these messages otherwise we will see them again.
                case "register":
                    message = RegisterMessage(**message_dict)
                    register_chid: str = message.channelID
                    record = self.register_records.get(register_chid, None)
                case "unregister":
                    message = UnregisterMessage(**message_dict)
                    unregister_chid: str = message.channelID
                    record = self.unregister_records.get(unregister_chid)
                case _:
                    exception = f"Unexpected data was received. Data: {data}"

            if record:
                response_time = (recv_time - record.send_time) * 1000
            else:
                if key and key in self.purged_records and message:
                    logger.error(
                        f"🔴Duplicate record {key} :: {cast(NotificationMessage, message).version}?"
                    )
                else:
                    exception = f"There is no record of the '{message_type}' message"
                    logger.error(f"{exception}. Contents: {message}")
        except (ValidationError, JSONDecodeError) as error:
            exception = str(error)

        self.environment.events.request.fire(
            request_type="WSS",
            name=f"{message_type} - recv",
            response_time=response_time,
            response_length=len(data.encode("utf-8")),
            exception=exception,
            context=self.context(),
        )

        return message

    def send_ack(self, ws: WebSocket, channel_id: str, version: str) -> None:
        """Send an 'ack' message to Autopush.

        After sending a notification, the client must also send an 'ack' to the server
        to confirm receipt.

        Parameters
        ----------
        ws: WebSocket
        channel_id : str
            Notification message channel ID
        version : str
            Notification message version

        Raises
        ------
            WebSocketException: Error raised by the WebSocket client
        """
        message_type: str = "ack"
        data: dict[str, Any] = dict(
            messageType=message_type,
            updates=[dict(channelID=channel_id, version=version)],
        )
        self.send(ws, message_type, data)

    def send_hello(self, ws: WebSocket) -> None:
        """Send a 'hello' message to Autopush.

        Connections must say hello after connecting to the server, otherwise the connection is
        quickly dropped.

        Parameters
        ----------
        ws : WebSocket
            Websocket class object

        Raises
        ------
        WebSocketException
            Error raised by the WebSocket client
        """
        message_type: str = "hello"
        data: dict[str, Any] = dict(
            messageType=message_type,
            use_webpush=True,
            uaid=self.uaid,
            channelIDs=list(self.channels.keys()),
        )
        self.hello_record = HelloRecord(send_time=time.perf_counter())
        self.send(ws, message_type, data)

    def send_register(self, ws: WebSocket, channel_id: str):
        """Send a 'register' message to Autopush.

        Parameters
        ----------
        ws : WebSocket
        channel_id : str
            Notification message channel ID

        Raises
        ------
        WebSocketException
        """
        message_type: str = "register"
        data: dict[str, Any] = dict(messageType=message_type, channelID=channel_id)
        record = RegisterRecord(send_time=time.perf_counter(), channel_id=channel_id)
        self.register_records[channel_id] = record
        self.send(ws, message_type, data)

    def send_unregister(self, ws: WebSocketApp, channel_id: str) -> None:
        """Send an 'unregister' message to Autopush.

        Parameters
        ----------
        ws : WebSocket
        channel_id : str
            Notification message channel ID

        Raises
        ------
        WebSocketException
        """
        message_type: str = "unregister"
        data: dict[str, Any] = dict(messageType=message_type, channelID=channel_id)
        record = RegisterRecord(send_time=time.perf_counter(), channel_id=channel_id)
        self.unregister_records[channel_id] = record
        self.send(ws, message_type, data)

    def send(self, ws: WebSocket | WebSocketApp, message_type: str, data: dict[str, Any]) -> None:
        """Send a message to Autopush.

        Parameters
        ----------
        ws : WebSocket
        message_type : str
            Examples: 'ack', 'hello', 'register' or 'unregister'
        data : dict[str, Any]
            Message data

        Raises
        ------
        WebSocketException
        """
        try:
            ws.send(json.dumps(data))
        except WebSocketConnectionClosedException as error:
            # WebSocket closures are expected
            # Don't send the exception to the UI it creates too much clutter
            logger.error(str(error))

        self.environment.events.request.fire(
            request_type="WSS",
            name=f"{message_type} - send",
            response_time=0,
            response_length=0,
            exception=None,
            context=self.context(),
        )
