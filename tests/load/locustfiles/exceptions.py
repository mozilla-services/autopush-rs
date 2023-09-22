# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.


class ZeroStatusRequestError(Exception):
    """Custom exception for when a Locust request fails with a '0' status code."""

    def __init__(self):
        error_message: str = (
            "A connection, timeout or similar error happened while sending a request "
            "from Locust. Status Code: 0"
        )
        super().__init__(error_message)
