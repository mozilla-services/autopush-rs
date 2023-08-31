# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

"""Performance test module."""

import json
import time
import uuid
from typing import Any

from locust import FastHttpUser, between, events, task
from locust.exception import StopUser
from models import HelloMessage, RegisterMessage
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

        if not isinstance(args[1], (AssertionError, type(None))):
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
            ssl=False,
        )

    def disconnect(self) -> None:
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
            body = json.dumps(dict(messageType="hello", use_webpush=True))
            self.ws.send(body)
            reply = self.ws.recv()
            assert reply, "No 'hello' response"
            res: HelloMessage = HelloMessage(**json.loads(reply))
            assert res.status == 200, f"Unexpected status. Expected: 200 Actual: {res.status}"
            timer.response_length = len(reply.encode("utf-8"))
        self.uaid = res.uaid

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
            assert reply, f"No 'register' response CHID: {chid}"
            res: RegisterMessage = RegisterMessage(**json.loads(reply))
            assert res.status == 200, f"Unexpected status. Expected: 200 Actual: {res.status}"
            assert res.channelID == chid, f"Channel ID did not match, received {res.channelID}"
            timer.response_length = len(reply.encode("utf-8"))
        self.channels[chid] = res.pushEndpoint

    @task
    def do_nothing(self) -> None:
        pass
