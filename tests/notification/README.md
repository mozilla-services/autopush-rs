# Autopush Notification Integration Tests

## About

Notifications in Firefox are a crucial part of its functionality. Firefox uses [autopush](https://github.com/mozilla-services/autopush) for this. This directory  contains a set of tests to check the functionality of these notifications.

## Technology

The tests use [Selenium](https://www.selenium.dev/), [pytest](https://docs.pytest.org/en/stable/index.html), [docker](https://www.docker.com/) as well as Firefox.

## Getting Started

Make sure you have installed [docker-compose](https://docs.docker.com/compose/) as well as Docker.

```sh
docker compose build
docker compose up server
NOTIFICATION_TEST_ENV="dev" docker compose run -it tests
```

You can also use the `Makefile` at the root of the project like so:
```sh
NOTIFICATION_TEST_ENV="stage" make notification-test
```

Be sure to run `make notification-test-clean` between successive test runs.

### Command line options

```NOTIFICATION_TEST_ENV``` : stage, dev, prod. This controls the URL that is set for the push server.
- stage: wss://autopush.stage.mozaws.net
- dev: wss://autopush.dev.mozaws.net/
- prod: wss://push.services.mozilla.com/
