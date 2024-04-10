# Testing

## Test Strategy

Autopush is tested using a combination of functional, integration, and performance tests.

Unit tests are written in the same Rust module as the code they are testing. Integration and Load Test code are in the `tests/` directory, both written in Python.

Presently, the Autopush test strategy does not require a minimum test coverage percentage for unit and integration tests. However, it is the goal that the service eventually have defined minimum coverage.
Load tests results should not go below a minimum performance threshold.

The functional test strategy is three-tiered, composed of: 

- [unit test dir][unit_tests] - [documentation][unit_tests_docs]
- [integration test dir][integration_tests] - [documentation][integration_tests_docs]
- [load test dir][load_tests] - [documentation][load_tests_docs]

See the documentation in each given test area for specific details on running and maintaining tests.

## Unit Tests

Unit tests allow for testing individual components of code in isolation to ensure they function as expected. Rust's built-in support for writing and running unit tests use the `#[cfg(test)]` attribute and the `#[test]` attribute.

### Best Practices

- Test functions are regular Rust functions annotated with the `#[test]` attribute.
- Test functions should be written in the same module as the code they are testing.
- Test functions should be named in a manner that describes the behavior being tested.

For example:

```Rust
    #[test]
    fn test_broadcast_change_tracker()
```

- The use of assertion macros is encouraged. This includes, but is not limited to:
`assert_eq!(actual, expected)`, `assert_ne!(actual, expected)`, `assert!(<condition>)`.
- You should group related tests into modules using the `mod` keyword. Furthermore, test modules can be nested to organize tests in a hierarchy.

### Running Unit Tests
Run Rust unit tests with the `cargo test` command from the root of the directory.

To run a specific test, provide the function name to `cargo test`. Ex. `cargo test test_function_name`.

## Integration Tests
The autopush-rs tests are written in Python and located in the [integration test directory][integration_tests]. 

### Testing Configuration
All dependencies are maintained by Poetry and defined in the `tests/pyproject.toml` file.  

There are a few configuration steps required to run the Python integration tests:

1. Depending on your operating system, ensure you have `cmake` and `openssl` installed. If using MacOS, for example, you can use `brew install cmake openssl`.
2. Build Autopush-rs: from the root directory, execute `cargo build`
3. Setup Local Bigtable emulator:

- Install the [Google Cloud CLI](https://cloud.google.com/sdk/gcloud)
- Install and run the [Google Bigtable Emulator](https://cloud.google.com/bigtable/docs/emulator)
- Configure the Bigtable emulator by running the following shell script: (***Note***, this will create a project and instance both named `test`, meaning that the tablename will be `projects/test/instances/test/tables/autopush`)

```bash
BIGTABLE_EMULATOR_HOST=localhost:8086 \
cbt -project test -instance test createtable autopush && \
cbt -project test -instance test createfamily autopush message && \
cbt -project test -instance test createfamily autopush message_topic && \
cbt -project test -instance test createfamily autopush router && \
cbt -project test -instance test setgcpolicy autopush message maxage=1s && \
cbt -project test -instance test setgcpolicy autopush router maxversions=1 && \
cbt -project test -instance test setgcpolicy autopush message_topic maxversions=1 and maxage=1s
```

4. Create Python virtual environment. It is recommended to use `pyenv virtualenv`:

```shell
$ pyenv virtualenv
$ pyenv install 3.12 # install matching version currently used
$ pyenv virtualenv 3.12 push-312 # you can name this whatever you like
$ pyenv local push-312 # sets this venv to activate when entering dir
$ pyenv activate push-312
```

5. Run `poetry install` to install all dependencies for testing.

### Running Integration Tests
To run the integration tests, simply run `make integration-tests` from your terminal at the root of the project.

You can alter the verbosity and logging output by adding command line flags to the `PYTEST_ARGS ?=` variable in the root project Makefile. For example, for greater verbosity and stdout printing, add `-vv -s`.

The test output is then emitted in your terminal instance. This includes the name of the tests, whether they pass or fail and any exceptions that are triggered during the test run.

Integration tests in CI will be triggered automatically whenever a commit is pushed to a branch as a part of the CI PR workflow.

### Debugging
In some instances after making test changes, the test client can potentially hang in a dangling process. This can result in inaccurate results or tests not running correctly. You can run the following commands to determine the PID's of the offending processes and terminate them:
```shell
$ ps -fA | grep autopush
# any result other than grep operation is dangling
$ kill -s KILL <PID>
```

## Firefox Testing

To test a locally running Autopush with Firefox, you will need to edit
several config variables in Firefox.

1. Open a New Tab.
2. Go to `about:config` in the Location bar and hit Enter, accept the
    disclaimer if it's shown.
3. Search for `dom.push.serverURL`, make a note of the existing value
    (you can right-click the preference and choose `Reset` to restore
    the default).
4. Double click the entry and change it to `ws://localhost:8080/`.
5. Right click in the page and choose `New -> Boolean`, name it
    `dom.push.testing.allowInsecureServerURL` and set it to `true`.

You should then restart Firefox to begin using your local Autopush.

### Debugging

On Android, you can set `dom.push.debug` to enable debug logging of Push
via `adb logcat`.

For desktop use, you can set `dom.push.loglevel` to `"debug"`. This will
log all push messages to the Browser Console (Tools \> Web Developer \>
Browser Console).

## Load Tests - Performance Testing

Performance load tests can be found under the `tests/load` directory. These tests spawn
multiple clients that connect to Autopush in order to simulate real-world load on the
infrastructure. These tests use the Locust framework and are triggered manually at the
discretion of the Autopush Engineering Team.

For more details see the [README.md][load_tests_docs] file in the `tests/load` directory.

[unit_tests]: https://github.com/mozilla-services/autopush-rs/tree/master/
[unit_tests_docs]: ./testing.md#unit-tests
[integration_tests]: https://github.com/mozilla-services/autopush-rs/tree/master/tests/integration
[integration_tests_docs]: ./testing.md#integration-tests
[load_tests]: https://github.com/mozilla-services/autopush-rs/tree/master/tests/load
[load_tests_docs]: https://github.com/mozilla-services/autopush-rs/blob/master/tests/load/README.md
[dynamo_deps]: https://docs.aws.amazon.com/amazondynamodb/latest/developerguide/DynamoDBLocal.DownloadingAndRunning.html