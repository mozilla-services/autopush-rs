from locust import between, constant


def parse_wait_time(val: str):
    """Parse a wait_time

    Either a single numeric (for `constant`) or two separated by a
    comma (arguments to `between`)

    """
    match val.count(","):
        case 0:
            return constant(float_or_int(val))
        case 1:
            return between(*map(float_or_int, val.split(",", 1)))
        case _:
            raise ValueError("Invalid wait_time")


def float_or_int(val: str):
    val = str(float(val))
    return val
