"""Conftest file for notification tests."""

import logging

import pytest
from selenium.webdriver.firefox.options import Options as FirefoxOptions
from selenium.webdriver.firefox.webdriver import WebDriver


def pytest_addoption(parser: pytest.Parser) -> None:
    """CLI Parser options."""
    parser.addoption("--env", action="store")


@pytest.fixture
def autopush_env(pytestconfig: pytest.Config) -> str:
    """Autopush websocket URLs."""
    environment = pytestconfig.getoption("env")
    urls: dict[str, str] = {
        "dev": "wss://autopush.dev.mozaws.net/",
        "stage": "wss://autopush.stage.mozaws.net",
        "prod": "wss://push.services.mozilla.com/",
    }
    logging.info(f"Testing ENVIRONMENT: {environment}")
    return urls.get(environment, "")


@pytest.fixture
def selenium(selenium: WebDriver) -> WebDriver:
    """Selenium setup fixture."""
    selenium.maximize_window()
    return selenium


@pytest.fixture
def firefox_options(firefox_options: FirefoxOptions, autopush_env: str) -> FirefoxOptions:
    """Selenium Firefox options fixture."""
    firefox_options.set_preference("dom.push.serverURL", autopush_env)
    firefox_options.add_argument("-foreground")
    return firefox_options
