"""Module containing Test Client for autopush-rs integration tests."""
import json
import logging
import random
import time
import uuid
from typing import Any
from urllib.parse import urlparse

import requests
import websocket
from twisted.internet.threads import deferToThread

logging.basicConfig(level=logging.DEBUG)
log = logging.getLogger(__name__)


class PushClient(object):
    """Push Client"""

    def __init__(self, url) -> None:
        self.url: str = url
        self.uaid: uuid.UUID | None = None
        self.ws: websocket.WebSocket | None = None
        self.use_webpush: bool = True
        self.channels: dict[str, str] = {}
        self.messages: dict[str, str] = {}
        self.notif_response: requests.Response | None = None
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

    def connect(self, connection_port: int | None = None):
        """Establish a websocket connection to localhost at the provided `connection_port`."""
        url = self.url
        if connection_port:  # pragma: nocover
            url = "ws://localhost:{}/".format(connection_port)
        self.ws = websocket.create_connection(url, header=self.headers)
        return self.ws.connected if self.ws else None

    def hello(self, uaid: str | None = None, services: list[str] | None = None):
        """Hello verification."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected")

        if self.channels:
            chans = list(self.channels.keys())
        else:
            chans = []
        hello_dict: dict[str, Any] = dict(messageType="hello", use_webpush=True, channelIDs=chans)
        if uaid or self.uaid:
            hello_dict["uaid"] = uaid or self.uaid
        if services:  # pragma: nocover
            hello_dict["broadcasts"] = services
        msg = json.dumps(hello_dict)
        log.debug("Send: %s", msg)
        self.ws.send(msg)
        reply = self.ws.recv()
        log.debug(f"Recv: {reply!r} ({len(reply)})")
        result = json.loads(reply)
        assert result["status"] == 200
        assert "-" not in result["uaid"]
        if self.uaid and self.uaid != result["uaid"]:  # pragma: nocover
            log.debug(
                "Mismatch on re-using uaid. Old: %s, New: %s",
                self.uaid,
                result["uaid"],
            )
            self.channels = {}
        self.uaid = result["uaid"]
        return result

    def broadcast_subscribe(self, services: list[str]):
        """Broadcast WebSocket subscribe."""
        if not self.ws:
            raise Exception("WebSocket client not available as expected")

        msg = json.dumps(dict(messageType="broadcast_subscribe", broadcasts=services))
        log.debug("Send: %s", msg)
        self.ws.send(msg)

    def register(self, chid: str | None = None, key=None, status=200):
        """Register a new endpoint for the provided ChannelID.
        Optionally locked to the provided VAPID Public key.
        """
        if not self.ws:
            raise Exception("WebSocket client not available as expected")

        chid = chid or str(uuid.uuid4())
        msg = json.dumps(dict(messageType="register", channelID=chid, key=key))
        log.debug("Send: %s", msg)
        self.ws.send(msg)
        rcv = self.ws.recv()
        result = json.loads(rcv)
        log.debug("Recv: %s", result)
        assert result["status"] == status
        assert result["channelID"] == chid
        if status == 200:
            self.channels[chid] = result["pushEndpoint"]
        return result

    def unregister(self, chid):
        """Unregister the ChannelID, which should invalidate the associated Endpoint."""
        msg = json.dumps(dict(messageType="unregister", channelID=chid))
        log.debug("Send: %s", msg)
        self.ws.send(msg)
        result = json.loads(self.ws.recv())
        log.debug("Recv: %s", result)
        return result

    def delete_notification(self, channel, message=None, status=204):
        """Delete notification."""
        messages = self.messages[channel]
        if not message:
            message = random.choice(messages)

        log.debug("Delete: %s", message)
        url = urlparse(message)
        resp = requests.delete(url=url.geturl())
        assert resp.status_code == status

    def send_notification(
        self,
        channel=None,
        version=None,
        data=None,
        use_header=True,
        status=None,
        ttl=200,
        timeout=0.2,
        vapid=None,
        endpoint=None,
        topic=None,
        headers=None,
    ):
        """Send notification."""
        if not channel:
            channel = random.choice(list(self.channels.keys()))

        endpoint = endpoint or self.channels[channel]
        url = urlparse(endpoint)

        headers = {}
        if ttl is not None:
            headers = {"TTL": str(ttl)}
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
            headers.update({"Authorization": "Bearer " + vapid.get("auth")})
            ckey = 'p256ecdsa="' + vapid.get("crypto-key") + '"'
            headers.update({"Crypto-Key": headers.get("Crypto-Key", "") + ";" + ckey})
        if topic:
            headers["Topic"] = topic
        body = data or ""
        method = "POST"
        # 202 status reserved for yet to be implemented push w/ reciept.
        status = status or 201

        log.debug("%s body: %s", method, body)
        log.debug("  headers: %s", headers)
        resp = requests.request(method=method, url=url.geturl(), data=body, headers=headers)
        log.debug("%s Response (%s): %s", method, resp.status_code, resp.text)
        assert resp.status_code == status, "Expected %d, got %d" % (
            status,
            resp.status_code,
        )
        self.notif_response = resp
        location = resp.headers.get("Location", None)
        log.debug("Response Headers: %s", resp.headers)
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
        # Pull the notification if connected
        if self.ws and self.ws.connected:
            return object.__getattribute__(self, "get_notification")(timeout)
        else:
            return resp

    def get_notification(self, timeout=1):
        """Get notification."""
        orig_timeout = self.ws.gettimeout()
        self.ws.settimeout(timeout)
        try:
            d = self.ws.recv()
            log.debug("Recv: %s", d)
            return json.loads(d)
        except Exception:
            return None
        finally:
            self.ws.settimeout(orig_timeout)

    def get_broadcast(self, timeout=1):  # pragma: nocover
        """Get broadcast."""
        orig_timeout = self.ws.gettimeout()
        self.ws.settimeout(timeout)
        try:
            d = self.ws.recv()
            log.debug("Recv: %s", d)
            result = json.loads(d)
            assert result.get("messageType") == "broadcast"
            return result
        except Exception as ex:  # pragma: nocover
            log.error("Error: {}".format(ex))
            return None
        finally:
            self.ws.settimeout(orig_timeout)

    def ping(self):
        """Test ping."""
        log.debug("Send: %s", "{}")
        self.ws.send("{}")
        result = self.ws.recv()
        log.debug("Recv: %s", result)
        assert result == "{}"
        return result

    def ack(self, channel, version):
        """Acknowledge message send."""
        msg = json.dumps(
            dict(
                messageType="ack",
                updates=[dict(channelID=channel, version=version)],
            )
        )
        log.debug("Send: %s", msg)
        self.ws.send(msg)

    def disconnect(self):
        """Disconnect from the application websocket."""
        self.ws.close()

    def sleep(self, duration: int):  # pragma: nocover
        """Sleep wrapper function."""
        time.sleep(duration)

    def wait_for(self, func):
        """Wait several seconds for a function to return True"""
        times = 0
        while not func():  # pragma: nocover
            time.sleep(1)
            times += 1
            if times > 9:  # pragma: nocover
                break


class CustomClient(PushClient):
    def send_bad_data(self):
        self.ws.send("bad-data")
