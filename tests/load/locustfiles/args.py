# This Source Code Form is subject to the terms of the Mozilla Public
# License, v. 2.0. If a copy of the MPL was not distributed with this
# file, You can obtain one at http://mozilla.org/MPL/2.0/.

from locust import between, constant


def parse_wait_time(val: str):
    """Parse a wait_time

    Either a single numeric (for `constant`) or two separated by a comma (arguments to `between`)
    """
    match val.count(","):
        case 0:
            return constant(float_or_int(val))
        case 1:
            return between(*map(float_or_int, val.split(",", 1)))
        case _:
            raise ValueError("Invalid wait_time")


def float_or_int(val: str):
    float_val: float = float(val)
    return int(float_val) if float_val.is_integer() else float_val
