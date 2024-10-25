"""Conftest file for notification tests."""

from typing import Any

import pytest


def pytest_addoption(parser: Any) -> None:
    """CLI Parser options."""
    parser.addoption("--env", action="store")


@pytest.fixture
def autopush_env(pytestconfig: Any) -> str:
    """Autopush websocket URLs."""
    url: str = ""
    if pytestconfig.getoption("env") == "stage":
        url = "wss://autopush.stage.mozaws.net"
    elif pytestconfig.getoption("env") == "dev":
        url = "wss://autopush.dev.mozaws.net/"
    elif pytestconfig.getoption("env") == "prod":
        url = "wss://push.services.mozilla.com/"
    return url


@pytest.fixture
def selenium(selenium: Any) -> Any:
    """Selenium setup fixture."""
    selenium.maximize_window()
    return selenium


@pytest.fixture
def firefox_options(firefox_options: Any, autopush_env: str) -> Any:
    """Selenium Firefox options fixture."""
    firefox_options.set_preference("dom.push.serverURL", autopush_env)
    firefox_options.add_argument("-foreground")
    return firefox_options
