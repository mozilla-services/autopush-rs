# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

"""Load test models module."""

from typing import Any, Literal

from pydantic import BaseModel


class HelloMessage(BaseModel):
    """Autopush 'hello' response message."""

    messageType: Literal["hello"]
    uaid: str
    status: Literal[200]
    use_webpush: bool
    broadcasts: dict[str, Any]


class HelloRecord(BaseModel):
    """Record of 'hello' message sent to Autopush."""

    send_time: float


class NotificationMessage(BaseModel):
    """Autopush 'notification' response message."""

    data: str
    headers: dict[str, str]
    messageType: Literal["notification"]
    channelID: str
    version: str


class NotificationRecord(BaseModel):
    """Record of 'notification' posted to Autopush."""

    send_time: float
    data: str
    location: str | None = None


class RegisterMessage(BaseModel):
    """Autopush 'register' response message."""

    messageType: Literal["register"]
    channelID: str
    status: Literal[200]
    pushEndpoint: str


class RegisterRecord(BaseModel):
    """Record of 'register' message sent to Autopush."""

    send_time: float
    channel_id: str


class UnregisterMessage(BaseModel):
    """Autopush 'unregister' response message."""

    messageType: Literal["unregister"]
    channelID: str
    status: Literal[200]
