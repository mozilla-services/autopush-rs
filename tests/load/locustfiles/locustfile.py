"""Performance test module."""
# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

import json
import logging
import random
import string
import time
import uuid
from hashlib import sha1
from json import JSONDecodeError
from logging import Logger
from typing import Any, TypeAlias

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


@events.test_start.add_listener
def _(environment, **kwargs):
    environment.autopush_wait_time = parse_wait_time(environment.parsed_options.wait_time)


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
        self.notification_records: dict[bytes, NotificationRecord] = {}
        self.register_records: dict[str, RegisterRecord] = {}
        self.unregister_records: dict[str, RegisterRecord] = {}
        self.uaid: str = ""
        self.ws: WebSocketApp | None = None
        self.ws_greenlet: Greenlet | None = None

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

    def on_ws_open(self, ws: WebSocket) -> None:
        """Call when opening a WebSocket.

        Parameters
        ----------
        ws : WebSocket
        """
        self.send_hello(ws)

    def on_ws_message(self, ws: WebSocket, data: str) -> None:
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
        elif isinstance(message, NotificationMessage):
            self.send_ack(ws, message.channelID, message.version)
        elif isinstance(message, RegisterMessage):
            self.channels[message.channelID] = message.pushEndpoint
        elif isinstance(message, UnregisterMessage):
            del self.channels[message.channelID]

    def on_ws_error(self, ws: WebSocket, error: Exception) -> None:
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
        self, ws: WebSocket, close_status_code: int | None, close_msg: str | None
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
        self.notification_records[sha1(data.encode()).digest()] = record  # nosec

        with self.client.post(
            url=endpoint_url,
            name=message_type,
            data=data,
            headers=self.REST_HEADERS,
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
            match message_type:
                case "hello":
                    message = HelloMessage(**message_dict)
                    record = self.hello_record
                case "notification":
                    message = NotificationMessage(**message_dict)
                    message_data: str = message.data
                    # scan through the notification records to see
                    # if this matches a record we sent.
                    record = self.notification_records.get(
                        sha1(message_data.encode()).digest(), None  # nosec
                    )
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
