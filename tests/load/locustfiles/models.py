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


class RegisterMessage(BaseModel):
    """Autopush 'register' response message."""

    messageType: Literal["register"]
    channelID: str
    status: Literal[200]
    pushEndpoint: str


class NotificationMessage(BaseModel):
    """Autopush 'ack' response message."""

    data: str
    headers: dict[str, str]
    messageType: Literal["notification"]
    channelID: str
    version: str
