"""Module containing Test Client for autopush-rs integration tests."""

import asyncio
import json
import logging
import random
import uuid
from enum import Enum
from typing import Any
from urllib.parse import urlparse

import httpx
import websockets
from websockets.exceptions import WebSocketException

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


class AsyncPushTestClient:
    """Push Test Client for integration tests."""

    def __init__(self, url) -> None:
        self.url: str = url
        self.uaid: uuid.UUID | None = None
        self.ws: websockets.WebSocketClientProtocol | None = None
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

    async def connect(self, connection_port: int | None = None) -> None:
        """Establish a websocket connection to localhost at the provided `connection_port`.

        Parameters
        ----------
        connection_port : int, optional
            A defined connection port, which is auto-assigned unless specified.
        """
        url: str = self.url
        if connection_port:  # pragma: no cover
            url = f"ws://localhost:{connection_port}/"
        self.ws = await websockets.connect(uri=url, extra_headers=self.headers)

    async def ws_server_send(self, message: dict) -> None:
        """Send message to websocket server.
        Serialize dictionary into a JSON object and send to server.

        Parameters
        ----------
        message : dict
            message content being sent to websocket server.
        """
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected.")
        payload: str = json.dumps(message)
        log.debug(f"Send: {payload}")
        await self.ws.send(payload)

    async def hello(self, uaid: str | None = None, services: list[str] | None = None):
        """Hello verification."""
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected.")

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

        await self.ws_server_send(message=hello_content)

        reply = await self.ws.recv()
        log.debug(f"Recv: {reply!r} ({len(reply)})")
        result = json.loads(reply)

        assert result["status"] == 200
        if self.uaid and self.uaid != result["uaid"]:  # pragma: no cover
            log.debug(f"Mismatch on re-using uaid. Old: {self.uaid}, New: {result['uaid']}")
            self.channels = {}
        self.uaid = result["uaid"]
        return result

    async def broadcast_subscribe(self, services: list[str]) -> None:
        """Broadcast WebSocket subscribe."""
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected.")

        message: dict = dict(
            messageType=ClientMessageType.BROADCAST_SUBSCRIBE.value, broadcasts=services
        )
        await self.ws_server_send(message=message)

    async def register(self, channel_id: str | None = None, key=None, status=200):
        """Register a new endpoint for the provided ChannelID.
        Optionally locked to the provided VAPID Public key.
        """
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected.")

        chid: str = channel_id or str(uuid.uuid4())
        message: dict = dict(messageType=ClientMessageType.REGISTER.value, channelID=chid, key=key)
        await self.ws_server_send(message=message)
        rcv = await self.ws.recv()
        result: Any = json.loads(rcv)
        log.debug(f"Recv: {result}")
        assert result["status"] == status
        assert result["channelID"] == chid
        if status == 200:
            self.channels[chid] = result["pushEndpoint"]
        return result

    async def unregister(self, chid) -> Any:
        """Unregister the ChannelID, which should invalidate the associated Endpoint."""
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected")

        message: dict = dict(messageType=ClientMessageType.UNREGISTER.value, channelID=chid)
        await self.ws_server_send(message=message)

        rcv = await self.ws.recv()
        result = json.loads(rcv)
        log.debug(f"Recv: {result}")
        return result

    async def delete_notification(self, channel, message=None, status=204) -> httpx.Response:
        """Sender (non-client) notification delete.
        From perspective of sender, not the client. Implements HTTP client to interact with
        notification.
        """
        messages = self.messages[channel]
        if not message:
            message = random.choice(messages)

        log.debug(f"Delete: {message}")
        url = urlparse(message)
        async with httpx.AsyncClient() as httpx_client:
            resp = await httpx_client.delete(url=url.geturl(), timeout=30)
        return resp

    async def send_notification(
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
        async with httpx.AsyncClient() as httpx_client:
            resp = await httpx_client.request(
                method=method, url=url.geturl(), content=body, headers=headers
            )
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
        if self.ws and self.ws.is_client:  # check back on this after integration
            res = await object.__getattribute__(self, "get_notification")(timeout)
            return res
        else:
            return resp

    async def get_notification(self, timeout=1):
        """Get most recent notification from websocket server.
        Typically called after a `send_notification` is sent from client to server.
        Method to recieve response from websocket.

        Includes ability to define a timeout to simulate latency.
        """
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected")

        try:
            d = await asyncio.wait_for(self.ws.recv(), timeout)
            log.debug(f"Recv: {d!r}")
            return json.loads(d)
        except Exception:
            return None

    async def get_broadcast(self, timeout=1):  # pragma: no cover
        """Get broadcast."""
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected")

        try:
            d = await asyncio.wait_for(self.ws.recv(), timeout)
            log.debug(f"Recv: {d}")
            result = json.loads(d)
            assert result.get("messageType") == ClientMessageType.BROADCAST.value
            return result
        except WebSocketException as ex:  # pragma: no cover
            log.error(f"Error: {ex}")
            return None

    async def moz_ping(self):
        """Send a very small message and await response."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        log.debug("Send: {}")
        await self.ws.send("{}")
        result = await self.ws.recv()
        log.debug(f"Recv: {result}")
        return result

    async def ping(self):
        """Test ping/pong request and respose of websocket.
        A ping may serve as a keepalive or as a check that the remote endpoint received.
        """
        if not self.ws:
            raise Exception("WebSocket client not available as expected.")

        log.debug("Sending Ping")
        ping_future = await self.ws.ping()
        return ping_future

    async def ack(self, channel, version) -> None:
        """Acknowledge message send."""
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected.")

        message: str = json.dumps(
            dict(
                messageType=ClientMessageType.ACK.value,
                updates=[dict(channelID=channel, version=version)],
            )
        )
        log.debug(f"Send: {message}")
        await self.ws.send(message)

    async def disconnect(self) -> None:
        """Disconnect from the application websocket."""
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected.")

        await self.ws.close()

    async def sleep(self, duration: int) -> None:  # pragma: no cover
        """Sleep wrapper function."""
        await asyncio.sleep(duration)

    async def send_bad_data(self) -> None:
        """Send `bad-data` as a string. Used in determining Sentry output
        in autoconnect to ensure error logs indicate disconnection.
        """
        if not self.ws:
            raise WebSocketException("WebSocket client not available as expected.")

        await self.ws.send("bad-data")

    async def wait_for(self, func) -> None:
        """Wait several seconds for a function to return True."""
        # This function currently not used for anything so may be removable.
        # However, it may have had historic value when dealing with latency.
        times = 0
        while not func():  # pragma: no cover
            await asyncio.sleep(1)
            times += 1
            if times > 9:  # pragma: no cover
                break
