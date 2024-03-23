"""Module containing Test Client for autopush-rs integration tests."""

import json
import logging
import random
import time
import uuid
from enum import Enum
from typing import Any
from urllib.parse import urlparse

import httpx
import websocket
from twisted.internet.threads import deferToThread

logging.basicConfig(level=logging.DEBUG)
log = logging.getLogger(__name__)


class ClientMessageType(Enum):
    """All variants for Message Type sent from client."""

    HELLO = "hello"
    REGISTER = "register"
    UNREGISTER = "unregister"
    BROADCAST = "broadcast"
    BROADCAST_SUBSCRIBE = "broadcast_subscribe"
    ACK = "ack"
    NACK = "nack"
    PING = "ping"


class PushTestClient:
    """Push Test Client for integration tests."""

    def __init__(self, url) -> None:
        self.url: str = url
        self.uaid: uuid.UUID | None = None
        self.ws: websocket.WebSocket | None = None
        self.use_webpush: bool = True
        self.channels: dict[str, str] = {}
        self.messages: dict[str, list[str]] = {}
        self.notif_response: httpx.Response | None = None
        self._crypto_key: str = """\
keyid="http://example.org/bob/keys/123";salt="XZwpw6o37R-6qoZjw6KwAw=="\
"""
        self.headers: dict[str, str] = {
            "User-Agent": "Mozilla/5.0 (Macintosh; Intel Mac OS X 10.13; rv:61.0) "
            "Gecko/20100101 Firefox/61.0"
        }

    def __getattribute__(self, name: str):
        """Turn functions into deferToThread functions."""
        # Python fun to turn all functions into deferToThread functions
        f = object.__getattribute__(self, name)
        if name.startswith("__"):
            return f

        if callable(f):
            return lambda *args, **kwargs: deferToThread(f, *args, **kwargs)
        else:
            return f

    def ws_server_send(self, message: dict) -> None:
        """Send message to websocket server.
        Serialize dictionary into a JSON object and send to server.

        Parameters
        ----------
        message : dict
            message content being sent to websocket server.
        """
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")
        payload: str = json.dumps(message)
        log.debug(f"Send: {payload}")
        self.ws.send(payload)

    def connect(self, connection_port: int | None = None) -> None:
        """Establish a websocket connection to localhost at the provided `connection_port`.

        Parameters
        ----------
        connection_port : int, optional
            A defined connection port, which is auto-assigned unless specified.
        """
        url = self.url
        if connection_port:  # pragma: no cover
            url = f"ws://localhost:{connection_port}/"
        self.ws = websocket.create_connection(url, header=self.headers)
        return None

    def hello(self, uaid: str | None = None, services: list[str] | None = None):
        """Hello verification."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        if self.channels:
            channels = list(self.channels.keys())
        else:
            channels = []
        hello_content: dict[str, Any] = dict(
            messageType=ClientMessageType.HELLO.value, use_webpush=True, channelIDs=channels
        )

        if uaid or self.uaid:
            hello_content["uaid"] = uaid or self.uaid

        if services:  # pragma: no cover
            hello_content["broadcasts"] = services

        self.ws_server_send(message=hello_content)
        reply = self.ws.recv()
        log.debug(f"Recv: {reply!r} ({len(reply)})")
        result = json.loads(reply)
        assert result["status"] == 200
        if self.uaid and self.uaid != result["uaid"]:  # pragma: no cover
            log.debug(f"Mismatch on re-using uaid. Old: {self.uaid}, New: {result['uaid']}")
            self.channels = {}
        self.uaid = result["uaid"]
        return result

    def broadcast_subscribe(self, services: list[str]) -> None:
        """Broadcast WebSocket subscribe."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        message: dict = dict(
            messageType=ClientMessageType.BROADCAST_SUBSCRIBE.value, broadcasts=services
        )
        self.ws_server_send(message=message)

    def register(self, channel_id: str | None = None, key=None, status=200):
        """Register a new endpoint for the provided ChannelID.
        Optionally locked to the provided VAPID Public key.
        """
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        chid: str = channel_id or str(uuid.uuid4())
        message: dict = dict(messageType=ClientMessageType.REGISTER.value, channelID=chid, key=key)
        self.ws_server_send(message=message)
        rcv = self.ws.recv()
        result: Any = json.loads(rcv)
        log.debug(f"Recv: {result}")
        assert result["status"] == status
        assert result["channelID"] == chid
        if status == 200:
            self.channels[chid] = result["pushEndpoint"]
        return result

    def unregister(self, chid) -> Any:
        """Unregister the ChannelID, which should invalidate the associated Endpoint."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected")

        message: dict = dict(messageType=ClientMessageType.UNREGISTER.value, channelID=chid)
        self.ws_server_send(message=message)
        result = json.loads(self.ws.recv())
        log.debug(f"Recv: {result}")
        return result

    def delete_notification(self, channel, message=None, status=204) -> httpx.Response:
        """Sender (non-client) notification delete.
        From perspective of sender, not the client. Implements HTTP client to interact with
        notification.
        """
        messages = self.messages[channel]
        if not message:
            message = random.choice(messages)

        log.debug(f"Delete: {message}")
        url = urlparse(message)
        resp = httpx.delete(url=url.geturl(), timeout=30)
        return resp

    def send_notification(
        self,
        channel=None,
        version=None,
        data: str | None = None,
        use_header: bool = True,
        status: int = 201,
        # 202 status reserved for yet to be implemented push w/ reciept.
        ttl: int = 200,
        timeout: float | int = 0.2,
        vapid: dict = {},
        endpoint: str | None = None,
        topic: str | None = None,
        headers: dict = {},
    ):
        """Sender (not-client) sent notification.
        Not part of responsibility of client but a subscribed sender.
        Calling from PushTestClient provides introspection of values in
        both client interface and the sender.
        """
        if not channel:
            channel = random.choice(list(self.channels.keys()))

        endpoint = endpoint or self.channels[channel]
        url = urlparse(endpoint)

        headers = {}
        if ttl is not None:
            headers.update({"TTL": str(ttl)})
        if use_header:
            headers.update(
                {
                    "Content-Type": "application/octet-stream",
                    "Content-Encoding": "aesgcm",
                    "Encryption": self._crypto_key,
                    "Crypto-Key": 'keyid="a1"; dh="JcqK-OLkJZlJ3sJJWstJCA"',
                }
            )
        if vapid:
            headers.update({"Authorization": f"Bearer {vapid.get('auth')}".rstrip()})
            ckey: str = f'p256ecdsa="{vapid.get("crypto-key")}"'
            headers.update({"Crypto-Key": f"{headers.get('Crypto-Key', '')};{ckey}"})
        if topic:
            headers["Topic"] = topic
        body: str = data or ""
        method: str = "POST"
        log.debug(f"{method} body: {body}")
        log.debug(f"  headers: {headers}")
        resp = httpx.request(method=method, url=url.geturl(), content=body, headers=headers)
        log.debug(f"{method} Response ({resp.status_code}): {resp.text}")
        assert resp.status_code == status, f"Expected {status}, got {resp.status_code}"
        self.notif_response = resp
        location = resp.headers.get("Location", None)
        log.debug(f"Response Headers: {resp.headers}")
        if status >= 200 and status < 300:
            assert location is not None
        if status == 201 and ttl is not None:
            ttl_header = resp.headers.get("TTL")
            assert ttl_header == str(ttl)
        if ttl != 0 and status == 201:
            assert location is not None
            if channel in self.messages:
                self.messages[channel].append(location)
            else:
                self.messages[channel] = [location]
        # Pull the sent notification immediately if connected.
        # Calls `get_notification` to get response from websocket.
        if self.ws and self.ws.connected:
            return object.__getattribute__(self, "get_notification")(timeout)
        else:
            return resp

    def get_notification(self, timeout=1):
        """Get most recent notification from websocket server.
        Typically called after a `send_notification` is sent from client to server.
        Method to recieve response from websocket.

        Includes ability to define a timeout to simulate latency.
        """
        if not self.ws:
            raise Exception("WebSocket client not available as expected")

        orig_timeout = self.ws.gettimeout()
        self.ws.settimeout(timeout)
        try:
            d = self.ws.recv()
            log.debug(f"Recv: {d}")
            return json.loads(d)
        except Exception:
            return None
        finally:
            self.ws.settimeout(orig_timeout)

    def get_broadcast(self, timeout=1):  # pragma: no cover
        """Get broadcast."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected")

        orig_timeout = self.ws.gettimeout()
        self.ws.settimeout(timeout)
        try:
            d = self.ws.recv()
            log.debug(f"Recv: {d}")
            result = json.loads(d)
            assert result.get("messageType") == ClientMessageType.BROADCAST.value
            return result
        except Exception as ex:  # pragma: no cover
            log.error(f"Error: {ex}")
            return None
        finally:
            self.ws.settimeout(orig_timeout)

    def ping(self):
        """Test ping."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        log.debug("Send: {}")
        self.ws.send("{}")
        result = self.ws.recv()
        log.debug("Recv: %s", result)
        # assert result == "{}"
        return result

    def ack(self, channel, version) -> None:
        """Acknowledge message send."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        message: str = json.dumps(
            dict(
                messageType=ClientMessageType.ACK.value,
                updates=[dict(channelID=channel, version=version)],
            )
        )
        log.debug(f"Send: {message}")
        self.ws.send(message)

    def disconnect(self) -> None:
        """Disconnect from the application websocket."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        self.ws.close()

    def sleep(self, duration: int) -> None:  # pragma: no cover
        """Sleep wrapper function."""
        time.sleep(duration)

    def send_bad_data(self) -> None:
        """Send `bad-data` as a string. Used in determining Sentry output
        in autoconnect to ensure error logs indicate disconnection.
        """
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        self.ws.send("bad-data")

    def wait_for(self, func):
        """Wait several seconds for a function to return True"""
        # This function currently not used for anything so may be removable.
        # However, it may have had historic value when dealing with latency.
        times = 0
        while not func():  # pragma: no cover
            time.sleep(1)
            times += 1
            if times > 9:  # pragma: no cover
                break
