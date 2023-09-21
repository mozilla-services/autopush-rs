# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

"""Performance test module."""

import base64
import json
import random
import string
import time
import uuid
from typing import Any

from args import parse_wait_time
from exceptions import ZeroStatusRequestError
from locust import FastHttpUser, events, task
from models import HelloMessage, NotificationMessage, RegisterMessage
from pydantic import ValidationError
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
        "--wait_time",
        type=str,
        env_var="AUTOPUSH_WAIT_TIME",
        help="AutopushUser wait time between tasks",
        default="20, 25",
    )


@events.test_start.add_listener
def _(environment, **kwargs):
    environment.autopush_wait_time = parse_wait_time(environment.parsed_options.wait_time)


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
            exception_type = args[0]
            exception_value = args[1]
            traceback = args[2]

            if not isinstance(exception_value, (AssertionError, ValidationError)):
                # An unexpected exception occurred stop the user.
                self.user.stop()
                return None

            # Assertion and Validation errors are expected exceptional outcomes should
            # a message received from Autopush be invalid.
            exception = exception_type, exception_value, traceback

        self.user.environment.events.request.fire(
            request_type="WSS",
            name=self.name,
            response_time=(end_time - float(str(self.start_time))) * 1000,
            response_length=self.response_length,
            exception=exception,
            context=self.user.context(),
        )


class AutopushUser(FastHttpUser):
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

    def wait_time(self):
        return self.environment.autopush_wait_time(self)

    def on_start(self) -> Any:
        self.connect()
        if not self.ws:
            self.stop()

        self.hello()
        if not self.uaid:
            self.stop()

        self.register()
        if not self.channels:
            self.stop()

    def on_stop(self) -> Any:
        if self.ws:
            self.disconnect()

    def _time_event(self, name: str) -> TimeEvent:
        return TimeEvent(self, name)

    def connect(self) -> None:
        self.ws = create_connection(
            self.environment.parsed_options.websocket_url,
            header={"Origin": "http://localhost:1337"},
            timeout=5,  # timeout defaults to None
        )

    def disconnect(self) -> None:
        if self.ws:
            self.ws.close()
        self.ws = None

    def hello(self) -> None:
        """Send a 'hello' message to Autopush.

        Connections must say hello after connecting to the server, otherwise the connection is
        quickly dropped.

        Raises:
            AssertionError: If the hello message response is empty or has an invalid status
            ValidationError: If the hello message schema is not as expected
        """
        body: str = json.dumps(
            dict(
                messageType="hello",
                use_webpush=True,
                uaid=self.uaid,
                channelIDs=list(self.channels.keys()),
            )
        )
        with self._time_event(name="hello - send") as timer:
            self.ws.send(body)

        with self._time_event(name="hello - recv") as timer:
            response: str = self.ws.recv()
            assert response, "No 'hello' response"
            message: HelloMessage = HelloMessage(**json.loads(response))
            timer.response_length = len(response.encode("utf-8"))

        if not self.uaid:
            self.uaid = message.uaid

    def ack(self) -> None:
        """Send an 'ack' message to push.

        After sending a notification, the client must also send an 'ack' to the server to
        confirm receipt. If there is a pending notification, this will try and receive it
        before sending an acknowledgement.

        Raises:
            AssertionError: If the notification message response is empty
            ValidationError: If the notification message schema is not as expected
        """
        with self._time_event(name="notification - recv") as timer:
            response = self.ws.recv()
            assert response, "No 'notification' response"
            message: NotificationMessage = NotificationMessage(**json.loads(response))
            timer.response_length = len(response.encode("utf-8"))

        body: str = json.dumps(
            dict(
                messageType="ack",
                updates=[dict(channelID=message.channelID, version=message.version)],
            )
        )
        with self._time_event(name="ack - send"):
            self.ws.send(body)

    def post_notification(self, endpoint_url) -> bool:
        """Send a notification to Autopush.

        Args:
            endpoint_url: A channel destination endpoint url
        Returns:
            bool: Flag indicating if the post notification request was successful
        Raises:
            ZeroStatusRequestError: In the event that Locust experiences a network issue while
                                    sending a notification.
        """
        with self.client.post(
            url=endpoint_url,
            name="notification",
            data=self.encrypted_data,
            headers=self.headers,
            catch_response=True,
        ) as response:
            if response.status_code == 0:
                raise ZeroStatusRequestError()
            if response.status_code != 201:
                response.failure(f"{response.status_code=}, expected 201, {response.text=}")
                return False
        return True

    def register(self) -> None:
        """Send a 'register' message to Autopush. Subscribes to an Autopush channel.

        Raises:
            AssertionError: If the register message response is empty or has an invalid status or
                            channel ID.
            ValidationError: If the register message schema is not as expected
        """

        chid: str = str(uuid.uuid4())
        body = json.dumps(dict(messageType="register", channelID=chid))

        with self._time_event(name="register - send"):
            self.ws.send(body)

        with self._time_event(name="register - recv") as timer:
            response: str = self.ws.recv()
            assert response, "No 'register' response"
            message: RegisterMessage = RegisterMessage(**json.loads(response))
            assert (
                message.channelID == chid
            ), f"Channel ID Error. Expected: {chid} Actual: {message.channelID}"
            timer.response_length = len(response.encode("utf-8"))

        self.channels[chid] = message.pushEndpoint

    @task(weight=95)
    def send_direct_notification(self):
        """Sends a notification to a registered endpoint while connected to Autopush."""
        endpoint_url = self.channels[random.choice(list(self.channels.keys()))]

        if not self.post_notification(endpoint_url):
            self.stop()

        self.ack()
