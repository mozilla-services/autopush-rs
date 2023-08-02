import base64
import json
import random
import string
import time
import uuid
from contextlib import closing
from urllib.parse import urljoin, urlparse

from locust import HttpUser, TaskSet, events, task
from websocket import create_connection
from websocket._exceptions import WebSocketTimeoutException

"""
History:
These tests pre-date the initial production release of Autopush and touch on
a number of predicted scenarios. They were originally crafted using no framework,
then ported to a number of more 'artesinal' style frameworks.

Autopush provides a passive update system called "Megaphone"/"Broadcast". A client
may either have registered endpoints to receive push notifications (active), or
may simply connect up to the push server to receive Broadcast updates (passive).

"""


@events.init_command_line_parser.add_listener
def _(parser):
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


class ConnectionTaskSet(TaskSet):
    """Create a fake "encrypted" message.

    The server doesn't care about encryption. It does, however, apply a
    base64 encoding to the data (this is because it's possible to send pure
    binary messages to the server, however, this never happens in reality.)
    We apply a standard header to the messages, and then apply 1-4K of
    padding. The max size we allow for a push message is 4K.

    """

    encrypted_data = base64.urlsafe_b64decode(
        "TestData"
        + "".join(
            [
                random.choice(string.ascii_letters + string.digits)
                for i in range(0, random.randrange(1024, 4096, 2) - 8)
            ]
        )
        + "=="
    )
    headers = {"TTL": "60", "Content-Encoding": "aes128gcm"}

    @task
    def test_basic(self):
        """Perform a "basic" transaction test.

        Desktop Autopush clients use a websocket connection to exchange
        JSON command and response messages. (See
        [Autopush HTTP Endpoints for Notifications]
        (https://mozilla-services.github.io/autopush-rs/http.html#push-service-http-api)
        for details).

        This tests an "active" style connection

        """

        # A Channel ID is how the client User Agent differentiates between various
        # Web App push notification recipients.
        channel_id = str(uuid.uuid4())

        # Create a connection to the Autoconnect server
        with closing(
            create_connection(
                self.user.environment.parsed_options.websocket_url,
                header={"Origin": "http://localhost:1337"},
                ssl=False,
            )
        ) as ws:
            # Connections must say hello after connecting to the server, otherwise
            # the connection is quickly dropped.
            body = json.dumps(dict(messageType="hello", use_webpush=True))
            ws.send(body)

            # The "hello" response also contains the UserAgent ID (UAID) for the
            # user agent. The value is random and will be reassigned on reconnection
            # for passive connections. This value is finalized when an connection
            # becomes active.
            res = json.loads(ws.recv())
            assert res["messageType"] == "hello"
            body = json.dumps(dict(messageType="register", channelID=channel_id))
            ws.send(body)
            res = json.loads(ws.recv())

            # NOTE: I believe that this is cruft from an earlier system. This condition
            # should just be replaced with
            # ```
            # endpoint_url = res["pushEndpoint"]
            # ```
            if self.user.environment.parsed_options.endpoint_url and (
                "dev" not in self.user.environment.parsed_options.websocket_url
            ):
                path = urlparse(res["pushEndpoint"]).path
                endpoint_url = urljoin(
                    self.user.environment.parsed_options.endpoint_url, path
                )
            else:
                endpoint_url = res["pushEndpoint"]

            # Send the test nonce message to the endpoint.
            # We should get this message via the autoconnect handler
            # shortly afterward
            start_time = time.time()
            endpoint_res = self.client.post(
                url=endpoint_url,
                name="ENDPOINT test_basic",
                data=self.encrypted_data,
                headers=self.headers,
            )
            assert (
                endpoint_res.status_code == 201
            ), f"status code was {endpoint_res.status_code}"
            res = json.loads(ws.recv())
            assert base64.urlsafe_b64decode(res["data"]) == self.encrypted_data
            end_time = time.time()

            # Send an "ack" message to make the server delete the message
            # Otherwise we would get the message re-sent to us on reconnect
            ws.send(
                json.dumps(dict(messageType="ack", updates=dict(channelID=channel_id)))
            )

        self.user.environment.events.request.fire(
            request_type="WSS",
            name="WEBSOCKET test_basic",
            response_time=int((end_time - start_time) * 1000),
            response_length=len(res),
            exception=None,
            context=None,
        )

    @task
    def test_basic_topic(self):
        """Test a basic message transaction using a "topic".

        "Topic" messages will replace prior, queued instances. A topic can be
        any UA defined, URL Safe base64 compliant string. Upon reconnection,
        a UA should only get one of each "topic" message that contains only the
        latest sent data.

        Topic messages are not terribly common, only about 10% of incoming messages
        use topics.
        """

        # A Channel ID is how the client User Agent differentiates between various
        # Web App push notification recipients.
        channel_id = str(uuid.uuid4())

        # Create a connection to the Autoconnect server.
        with closing(
            create_connection(
                self.user.environment.parsed_options.websocket_url,
                header={"Origin": "http://localhost:1337"},
                ssl=False,
            )
        ) as ws:
            # Connections must say hello after connecting to the server, otherwise
            # the connection is quickly dropped.
            body = json.dumps(dict(messageType="hello", use_webpush=True))
            ws.send(body)

            # The "hello" response also contains the UserAgent ID (UAID) for the
            # user agent. The value is random and will be reassigned on reconnection
            # for passive connections. This value is finalized when an connection
            # becomes active.
            res = json.loads(ws.recv())
            assert res["messageType"] == "hello"
            uaid = res["uaid"]

            # Register an endpoint for delivery of the message.
            body = json.dumps(dict(messageType="register", channelID=channel_id))
            ws.send(body)
            res = json.loads(ws.recv())

            # NOTE: We are disconnecting from the Autoconnect server so that
            # we can potentially send multiple topic messages. If we were still
            # connected the server would deliver the messages as it received them.
            # That is to be expected.

        # NOTE: I believe that this is cruft from an earlier system. This condition
        # should just be replaced with
        # ```
        # endpoint_url = res["pushEndpoint"]
        # ```
        if self.user.environment.parsed_options.endpoint_url and (
            "dev" not in self.user.environment.parsed_options.websocket_url
        ):
            path = urlparse(res["pushEndpoint"]).path
            endpoint_url = urljoin(
                self.user.environment.parsed_options.endpoint_url, path
            )
        else:
            endpoint_url = res["pushEndpoint"]

        # The topic is specified by an expicit "Topic header."
        self.headers.update({"Topic": "aaaa"})

        # Send the test topic nonce message to the endpoint.
        # We should get this message via the autoconnect handler
        # after we reconnect.
        endpoint_res = self.client.post(
            url=endpoint_url,
            name="ENDPOINT test_basic_topic",
            data=self.encrypted_data,
            headers=self.headers,
        )
        assert (
            endpoint_res.status_code == 201
        ), f"status code was {endpoint_res.status_code}"

        # NOTE: To properly test "topic" messages, we really ought to
        # send 1 to 1+n "topics" before reconnecting and checking that
        # only the latest topic content was sent.

        # connect and check for notifications
        with closing(
            create_connection(
                self.user.environment.parsed_options.websocket_url,
                header={"Origin": "http://localhost:1337"},
                ssl=False,
                timeout=60,
            )
        ) as ws:
            start_time = time.time()
            # After we reconnect and say "Hello", we should start getting
            # any pending messages.
            body = json.dumps(dict(messageType="hello", use_webpush=True, uaid=uaid))
            ws.send(body)
            res = json.loads(ws.recv())
            assert res["messageType"] == "hello"
            msg = json.loads(ws.recv())

            # check that the data we're getting matches up with the data that
            # we sent, after decode.
            assert base64.urlsafe_b64decode(msg["data"]) == self.encrypted_data
            end_time = time.time()

            # Send an "ack" message to make the server delete the message
            # Otherwise we would get the message re-sent to us on reconnect
            ws.send(
                json.dumps(dict(messageType="ack", updates=dict(channelID=channel_id)))
            )
        self.user.environment.events.request.fire(
            request_type="WSS",
            name="WEBSOCKET test_basic_topic",
            response_time=int((end_time - start_time) * 1000),
            response_length=len(res),
            exception=None,
            context=None,
        )

    @task
    def test_connect_and_hold(self):
        """Create a "passive" connection.

        A client that is purely "passive" is only provided a temporary
        User Agent ID (UAID), which is discarded after the client disconnects.
        """

        # Create a connection to the Autoconnect server
        with closing(
            create_connection(
                self.user.environment.parsed_options.websocket_url,
                header={"Origin": "http://localhost:1337"},
                ssl=False,
            )
        ) as ws:
            start_time = time.time()

            # Connections must say hello after connecting to the server, otherwise
            # the connection is quickly dropped.
            body = json.dumps(dict(messageType="hello", use_webpush=True))
            ws.send(body)
            res = json.loads(ws.recv())
            assert res["messageType"] == "hello"
            end_time = time.time()
            self.user.environment.events.request.fire(
                request_type="WSS",
                name="WEBSOCKET test_connect_and_hold",
                response_time=int((end_time - start_time) * 1000),
                response_length=len(res),
                exception=None,
                context=None,
            )
            # NOTE: we should check that "broadcast" messages are
            # received. A broadcast message is a Ping that contains
            # a payload of IDs.
            time.sleep(30)

    @task
    def test_connect(self):
        """
        Create a simple connection to the autoconnect server.

        That's it. That's what it does.

        """
        with closing(
            create_connection(
                self.user.environment.parsed_options.websocket_url,
                header={"Origin": "http://localhost:1337"},
                ssl=False,
            )
        ) as ws:
            start_time = time.time()
            body = json.dumps(dict(messageType="hello", use_webpush=True))
            ws.send(body)
            res = json.loads(ws.recv())
            assert res["messageType"] == "hello"
            end_time = time.time()
            self.user.environment.events.request.fire(
                request_type="WSS",
                name="WEBSOCKET test_connect",
                response_time=int((end_time - start_time) * 1000),
                response_length=len(res),
                exception=None,
                context=None,
            )

    @task
    def test_connect_stored(self):
        """
        Send and recieve 10 topic messages to the endpoint server
        ensuring that they are stored. We should only get one message
        back.

        """
        channel_id = str(uuid.uuid4())

        # Connect and register to get a unique endpoint.
        with closing(
            create_connection(
                self.user.environment.parsed_options.websocket_url,
                header={"Origin": "http://localhost:1337"},
                ssl=False,
            )
        ) as ws:
            body = json.dumps(dict(messageType="hello", use_webpush=True))
            ws.send(body)
            res = json.loads(ws.recv())
            assert res["messageType"] == "hello"
            uaid = res["uaid"]
            body = json.dumps(dict(messageType="register", channelID=channel_id))
            ws.send(body)
            res = json.loads(ws.recv())
            # At the closure of this block, the connection should drop.

        # NOTE: I believe that this is cruft from an earlier system. This condition
        # should just be replaced with
        # ```
        # endpoint_url = res["pushEndpoint"]
        # ```
        if self.user.environment.parsed_options.endpoint_url and (
            "dev" not in self.user.environment.parsed_options.websocket_url
        ):
            path = urlparse(res["pushEndpoint"]).path
            endpoint_url = urljoin(
                self.user.environment.parsed_options.endpoint_url, path
            )
        else:
            endpoint_url = res["pushEndpoint"]

        # Set the "Topic" header. Topic messages replace prior messages
        # with a matching topic. Only the last Topic message should be
        # returned.
        self.headers.update({"Topic": "abcd"})
        for _ in range(10):
            endpoint_res = self.client.post(
                url=endpoint_url,
                name="ENDPOINT test_connect_stored",
                data=self.encrypted_data,
                headers=self.headers,
            )
            assert (
                endpoint_res.status_code == 201
            ), f"status code was {endpoint_res.status_code}"
        ws.close()

        # connect and check notification
        msg_count = 0
        exception = None

        # Connect to the server 10 times. This should return
        # the topic message once, for the first connection, provided
        # the message was ACK'd after receipt.
        # NOTE: As written, this test should fail with 9 of the
        # instances not getting a message.
        for _ in range(10):
            try:
                with closing(
                    create_connection(
                        self.user.environment.parsed_options.websocket_url,
                        header={"Origin": "http://localhost:1337"},
                        ssl=False,
                        timeout=30,
                    )
                ) as ws:
                    start_time = time.time()
                    body = json.dumps(
                        dict(messageType="hello", use_webpush=True, uaid=uaid)
                    )
                    ws.send(body)
                    res = json.loads(ws.recv())
                    assert res["messageType"] == "hello"
                    msg = json.loads(ws.recv())
                    assert msg["data"]
                    msg_count += 1
                    end_time = time.time()
                    ws.send(
                        json.dumps(
                            dict(messageType="ack", updates=dict(channelID=channel_id))
                        )
                    )
            except WebSocketTimeoutException as e:
                end_time = time.time()
                exception = e
            finally:
                self.user.environment.events.request.fire(
                    request_type="WSS",
                    name="WEBSOCKET test_connect_stored",
                    response_time=int((end_time - start_time) * 1000),
                    response_length=len(res),
                    exception=exception,
                    context=None,
                )
                ws.close()
        assert msg_count == 10

    @task
    def test_connect_forever(self):
        """
        Go from an active subscription to a passive subscription.

        The UAID ought to still remain valid, although the server can
        replace the UAID at any time if there are no outstanding
        subscriptions.

        """

        # A Channel ID is how the client User Agent differentiates between various
        # Web App push notification recipients.
        channel_id = str(uuid.uuid4())

        # Create a connection to the Autoconnect server
        ws = create_connection(
            self.user.environment.parsed_options.websocket_url,
            header={"Origin": "http://localhost:1337"},
            ssl=False,
        )

        # Connections must say hello after connecting to the server, otherwise
        # the connection is quickly dropped.
        body = json.dumps(dict(messageType="hello", use_webpush=True))
        ws.send(body)

        # The "hello" response also contains the UserAgent ID (UAID) for the
        # user agent. The value is random and will be reassigned on reconnection
        # for passive connections. This value is finalized when an connection
        # becomes active.
        res = json.loads(ws.recv())
        assert res["messageType"] == "hello"
        uaid = res["uaid"]
        body = json.dumps(dict(messageType="register", channelID=channel_id))
        ws.send(body)
        res = json.loads(ws.recv())

        # NOTE: I believe that this is cruft from an earlier system. This condition
        # should just be replaced with
        # ```
        # endpoint_url = res["pushEndpoint"]
        # ```
        if self.user.environment.parsed_options.endpoint_url and (
            "dev" not in self.user.environment.parsed_options.websocket_url
        ):
            path = urlparse(res["pushEndpoint"]).path
            endpoint_url = urljoin(
                self.user.environment.parsed_options.endpoint_url, path
            )
        else:
            endpoint_url = res["pushEndpoint"]

        # NOTE: Not sure why we're specifying a Topic here, but sure...?
        self.headers.update({"Topic": "zyxw"})
        while True:
            # NOTE: This feels odd.
            # We send a notification to the client, but then immediately
            # drop the websocket connection. There's a small chance
            # that the server already tried to deliver the message, but we
            # are not ACK'ing it, so the server will simply retry on next
            # connection. Why not drop the connection before we send?
            endpoint_res = self.client.post(
                url=endpoint_url,
                name="ENDPOINT test_connect_forever",
                data=self.encrypted_data,
                headers=self.headers,
            )
            assert (
                endpoint_res.status_code == 201
            ), f"status code was {endpoint_res.status_code}"
            ws.close()

            # sit on our thumbs for 15 seconds.
            time.sleep(15)

            # and then reconnect, but don't check if the response contains
            # the previously sent Topic message, but send an Ack anyway? 🤨
            ws = create_connection(
                self.user.environment.parsed_options.websocket_url,
                header={"Origin": "http://localhost:1337"},
                ssl=False,
            )
            body = json.dumps(dict(messageType="hello", use_webpush=True, uaid=uaid))
            ws.send(body)
            res = json.loads(ws.recv())
            assert res["messageType"] == "hello"
            ws.recv()
            ws.send(
                json.dumps(dict(messageType="ack", updates=dict(channelID=channel_id)))
            )

            ws.close()
            break

    # Hold a notification
    @task
    def test_notification_forever_unsubscribed(self):
        """
        Create an "active" connection, that we immediately turn "passive", then hold
        open for a period of time.
        """

        # A Channel ID is how the client User Agent differentiates between various
        # Web App push notification recipients.
        channel_id = str(uuid.uuid4())

        # Create a connection to the Autoconnect server
        with closing(
            create_connection(
                self.user.environment.parsed_options.websocket_url,
                header={"Origin": "http://localhost:1337"},
                ssl=False,
            )
        ) as ws:
            # Connections must say hello after connecting to the server, otherwise
            # the connection is quickly dropped.
            body = json.dumps(dict(messageType="hello", use_webpush=True))
            ws.send(body)
            res = json.loads(ws.recv())
            assert res["messageType"] == "hello"

            # Create an endpoint. This locks in our UAID.
            body = json.dumps(dict(messageType="register", channelID=channel_id))
            ws.send(body)
            res = json.loads(ws.recv())
            endpoint_url = res["pushEndpoint"]

            # Now discard the just created endpoint for some reason?
            body = json.dumps(dict(messageType="unregister", channelID=channel_id))
            ws.send(body)
            while True:
                # Send a Ping message with arbitrary text. This should result
                # in a Broadcast message response.
                ws.ping("hello")

                # Send a push message to ourselves using the freshly invalid
                # endpoint, and ensure that it's rejected.
                with self.client.post(
                    url=endpoint_url,
                    name="ENDPOINT test_notification_forever_unsubscribed",
                    data=self.encrypted_data,
                    headers=self.headers,
                    catch_response=True,
                ) as response:
                    if response.status_code == 410:
                        response.success()
                    else:
                        response.failure()

                # Try reading from the websocket. If it fails, send another
                # invalid message to our endpoint and try again. WTF? 🤨
                try:
                    ws.recv()
                except BrokenPipeError:
                    continue

                # If we got anything back send an ack with no list of values
                # to ack, then take a little nap for yourself before dropping.
                ws.send(json.dumps(dict(messageType="ack")))
                time.sleep(30)
                break


class LocustRunner(HttpUser):
    tasks = [ConnectionTaskSet]
    host = "https://updates-autopush.stage.mozaws.net"
