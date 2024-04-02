# Testing

## Test Strategy

Autopush is tested using a combination of functional, integration, and performance tests.

Unit tests are written in the same Rust module as the code they are testing. Integration and Load Test code are in the `tests/` directory.

Presently, the Autopush test strategy does not require a minimum test coverage percentage for unit and integration tests. However, it is the goal that the service eventually have defined minimum coverage.
Load tests should not go below a minimum performance threshold.

Test documentation resides in this document [/testing/][test_docs_dir] directory.

The functional test strategy is three-tiered, composed of: 

- [unit][unit_tests] - [documentation][unit_tests_docs]
- [integration][integration_tests] - [documentation][integration_tests_docs]
- [load][load_tests] - [documentation][load_tests_docs]

See the documentation in each given test area for specific details on running and maintaining tests.

[unit_tests_docs]: ./testing.md#unit-tests
[integration_tests]: https://github.com/mozilla-services/autopush-rs/tree/master/tests/integration
[integration_tests_docs]: ./testing.md#integration-tests
[load_tests]: https://github.com/mozilla-services/autopush-rs/tree/master/tests/load
[load_tests_docs]: https://github.com/mozilla-services/autopush-rs/blob/master/tests/load/README.md

## Unit Tests
Unit tests allow for testing individual components of code in isolation to ensure they function as expected. Rust's built-in support for writing and running unit tests use the `#[cfg(test)]` attribute and the `#[test]` attribute.

### Running Unit Tests
Run Rust unit tests with the `cargo test` comand from the root of the directory.

## Integration Tests

## Testing Configuration

When testing, it's important to reduce the number of potential conflicts
as much as possible. To that end, it's advised to have as clean a
testing environment as possible before running tests.

This includes:

* Making sure notifications are not globally blocked by your browser.
* "Do Not Disturb" or similar "distraction free" mode is disabled on
    your OS
* You run a "fresh" Firefox profile (start `firefox --P` to display the profile picker)
    which does not have extra extensions or optional plug-ins running.

You may find it useful to run firefox in a Virtual Machine (like
VirtualBox or VMWare), but this is not required.

In addition, it may be useful to open the Firefox Brower Console
(Ctrl+Shift+J) as well as the Firefox Web Console (Ctrl+Shift+K). Both
are located under the **Web Developer** sub-menu.

## Running Tests

If you plan on doing development and testing, you will need to install
some additional packages.

``` bash
$ bin/pip install -r test-requirements.txt
```

Once the Makefile has been run, you can run `make test` to run the test
suite.

<b>Note</b> Failures may occur if a `.boto` file exists in your home directory. This
file should be moved elsewhere before running the tests.`

### Disabling Integration Tests

`make test` runs the `tox` program which can be difficult to break for
debugging purposes. The following bash script has been useful for
running tests outside of tox:

``` bash
#! /bin/bash
mv autopush/tests/test_integration.py{,.hold}
mv autopush/tests/test_logging.py{,.hold}
bin/nosetests -sv autopush
mv autopush/tests/test_integration.py{.hold,}
mv autopush/tests/test_logging.py{.hold,}
```

This script will cause the integration and logging tests to not run.

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

### Performance Testing

Performance load tests can be found under the `tests/load` directory. These tests spawn
multiple clients that connect to Autopush in order to simulate real-world load on the
infrastructure. These tests use the Locust framework and are triggered manually at the
discretion of the Autopush Engineering Team.

For more details see the README.md file in the `tests/load` directory.
