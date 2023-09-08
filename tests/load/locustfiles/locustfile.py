# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

"""Performance test module."""

import base64
import json
import random
import ssl
import string
import time
import uuid
from enum import Enum, unique
from typing import Any

from locust import FastHttpUser, between, events, task
from locust.exception import StopUser
from models import HelloMessage, NotificationMessage, RegisterMessage
from websocket import create_connection


@events.init_command_line_parser.add_listener
def _(parser: Any):
    parser.add_argument(
        "--websocket_url",
        type=str,
        env_var="AUTOPUSH_WEBSOCKET_URL",
        required=True,
        help="Server URL",
    )
    parser.add_argument(
        "--endpoint_url",
        type=str,
        env_var="AUTOPUSH_ENDPOINT_URL",
        required=True,
        help="Endpoint URL",
    )
    parser.add_argument(
        "--notification_type",
        type=NotificationType,
        env_var="AUTOPUSH_NOTIFICATION_TYPE",
        help="Type of notification to send (direct or stored)",
        choices=list(NotificationType),
        default=NotificationType.STORED,
    )


@unique
class NotificationType(str, Enum):
    DIRECT = "direct"
    STORED = "stored"

    def __str__(self):
        return self.value


class TimeEvent:
    def __init__(self, user: FastHttpUser, name: str) -> None:
        self.user: FastHttpUser = user
        self.start_time: float
        self.name: str = name

    def __enter__(self) -> object:
        self.start_time = time.perf_counter()
        self.response_length: int = 0
        return self

    def __exit__(self, *args) -> None:
        end_time: float = time.perf_counter()
        exception: Any = None
        if args[0] is not None:
            exception = args[0], args[1]

        if not isinstance(args[1], (AssertionError, ssl.SSLEOFError, type(None))):
            self.user.environment.events.user_error.fire(
                user_instance=self.user.context(), exception=args[1], tb=args[2]
            )
            exception = None
        self.user.environment.events.request.fire(
            request_type="WSS",
            name=self.name,
            response_time=(end_time - float(str(self.start_time))) * 1000,
            response_length=self.response_length,
            exception=exception,
            context=self.user.context(),
        )


class AutopushUser(FastHttpUser):
    wait_time = between(30, 35)

    def __init__(self, environment) -> None:
        super().__init__(environment)
        self.uaid: str = ""
        self.channels: dict[str, str] = {}
        self.ws: Any = None
        self.headers = {"TTL": "60", "Content-Encoding": "aes128gcm", "Topic": "aaaa"}
        self.encrypted_data = base64.urlsafe_b64decode(
            "TestData"
            + "".join(
                [
                    random.choice(string.ascii_letters + string.digits)
                    for i in range(0, random.randrange(1024, 4096, 2) - 8)
                ]
            )
            + "=="
        )
        self.notifications: int = 0

    def on_start(self) -> Any:
        self.connect()
        if not self.ws:
            raise StopUser()
        self.hello()
        if not self.uaid:
            raise StopUser()
        self.register()
        if not self.channels:
            raise StopUser()

    def on_stop(self) -> Any:
        if self.ws:
            self.disconnect()

    def _time_event(self, name: str) -> TimeEvent:
        return TimeEvent(self, name)

    def connect(self) -> None:
        self.ws = create_connection(
            self.environment.parsed_options.websocket_url,
            header={"Origin": "http://localhost:1337"},
        )

    def disconnect(self) -> None:
        if self.ws:
            self.ws.close()

    def hello(self) -> None:
        """
        Send a 'hello' message to Autopush.

        Connections must say hello after connecting to the server, otherwise the connection is
        quickly dropped.

        Raises:
            AssertionError: If the user fails to send the hello
            ValidationError: If the hello message schema is not as expected
        """
        with self._time_event(name="hello") as timer:
            body = json.dumps(
                dict(
                    messageType="hello",
                    use_webpush=True,
                    uaid=self.uaid,
                    channelIDs=list(self.channels.keys()),
                )
            )
            self.ws.send(body)
            reply = self.ws.recv()
            assert reply, "No 'hello' response"
            res: HelloMessage = HelloMessage(**json.loads(reply))
            assert res.status == 200, f"Unexpected status. Expected: 200 Actual: {res.status}"
            timer.response_length = len(reply.encode("utf-8"))
        self.uaid = res.uaid

    def ack(self) -> None:
        """
        Send an 'ack' message to push.

        After sending a notification, the client must also send an 'ack' to the server to
        confirm receipt. If there is a pending notification, this will try and receive it
        before sending an acknowledgement.
        """

        with self._time_event(name="acknowledge") as timer:
            reply = self.ws.recv()
            notification = NotificationMessage(**json.loads(reply))
            body = json.dumps(
                dict(
                    messageType="ack",
                    updates=[dict(channelID=notification.channelID, version=notification.version)],
                )
            )
            self.ws.send(body)
            timer.response_length = len(reply.encode("utf-8"))

    @task(weight=3)
    def register(self) -> None:
        """
        Send a 'register' message to Autopush. Subscribes to an Autopush channel.

        Raises:
            AssertionError: If the user fails to register a channel
            ValidationError: If the register message schema is not as expected
        """

        chid: str = str(uuid.uuid4())

        with self._time_event(name="register") as timer:
            body = json.dumps(dict(messageType="register", channelID=chid))
            self.ws.send(body)
            reply = self.ws.recv()
            res: RegisterMessage = RegisterMessage(**json.loads(reply))
            assert res.status == 200, f"Unexpected status. Expected: 200 Actual: {res.status}"
            assert res.channelID == chid, f"Channel ID did not match, received {res.channelID}"
            timer.response_length = len(reply.encode("utf-8"))
            self.channels[chid] = res.pushEndpoint

    @task(weight=95)
    def send_notification(self):
        """
        Sends a notification to a registered enpoint

        Raises:
            AssertionError: If the server does not respond correctly (400, 500, etc)
        """
        stored = self.environment.parsed_options.notification_type == NotificationType.STORED
        if stored:
            self.disconnect()

        channel_id = random.choice(list(self.channels.keys()))

        with self._time_event(name="send_notification") as timer:
            endpoint_url = self.channels[channel_id]
            endpoint_res = self.client.post(
                url=endpoint_url,
                name="Endpoint Notification",
                data=self.encrypted_data,
                headers=self.headers,
            )
            assert endpoint_res.status_code == 201, f"status code was {endpoint_res.status_code}"
            self.notifications += 1
            timer.response_length = len(endpoint_res.text.encode("utf-8"))

        if stored:
            self.connect()
            self.hello()

        self.ack()
