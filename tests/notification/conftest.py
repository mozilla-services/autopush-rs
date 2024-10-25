import pytest


def pytest_addoption(parser):
    parser.addoption("--env", action="store")


@pytest.fixture
def autopush_env(pytestconfig):
    if pytestconfig.getoption("env") == "stage":
        return "wss://autopush.stage.mozaws.net"
    elif pytestconfig.getoption("env") == "dev":
        return "wss://autopush.dev.mozaws.net/"
    elif pytestconfig.getoption("env") == "prod":
        return "wss://push.services.mozilla.com/"


@pytest.fixture
def selenium(selenium):
    selenium.maximize_window()
    return selenium


@pytest.fixture
def firefox_options(firefox_options, autopush_env):
    firefox_options.set_preference("dom.push.serverURL", autopush_env)
    firefox_options.add_argument('-foreground')
    return firefox_options
