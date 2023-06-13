# Testing

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
