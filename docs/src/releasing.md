Release Process {#releasing}
===============

<div style="color:red;border:2px solid red;padding:5px; margin:10px 5px ">
<b><i>NOTE</i></b> This page is outdated, however the "Release Steps" are still a useful checklist.
</div>

Autopush has a regular 2-3 week release to production depending on
developer and QA availability. The developer creating a release should
handle all aspects of the following process as they\'re done closely in
order and time.

Versions
--------

Autopush uses a `{major}.{minor}.{patch}` version scheme, new `{major}`
versions are only issued if backwards compatibility is affected. Patch
versions are used if a critical bug occurs after production deployment
that requires a bug fix immediately.

Dev Releases
------------

When changes are committed to the `master` branch, an operations Jenkins
instance will build and deploy the code automatically to the dev
environment.

The development environment can be verified at its endpoint/wss
endpoints:

* Websocket: <wss://autopush.dev.mozaws.net/>
* Endpoint: <https://updates-autopush.dev.mozaws.net/>

Stage/Production Releases
-------------------------

### Pre-Requisites

To create a release, you will need appropriate access to the autopush
GitHub repository with push permission.

You will also need [clog](https://github.com/clog-tool/clog-cli)
installed to create the `CHANGELOG.md` update.

### Release Steps

In these steps, the `{version}` refers to the full version of the
release.

i.e. If a new minor version is being released after `1.21.0`, the
`{version}` would be `1.22.0`.

1. Switch to the `master` branch of autopush.
2. `git pull` to ensure the local copy is completely up-to-date.
3. `git diff origin/master` to ensure there are no local staged or
    uncommited changes.
4. Run `tox` locally to ensure no artifacts or other local changes that
    might break tests have been introduced.
5. Change to the release branch.

    If this is a new major/minor release,
    `git checkout -b release/{major}.{minor}` to create a new release
    branch.

    If this is a new patch release, you will first need to ensure you
    have the minor release branch checked out, then:

    > 1.  `git checkout release/{major}.{minor}`
    > 2.  `git pull` to ensure the branch is up-to-date.
    > 3.  `git merge master` to merge the new changes into the release
    >     branch.

    **Note that the release branch does not include a \`\`{patch}\`\`
    component**.

6. Edit `autopush/__init__.py` so that the version number reflects the
    desired release version.
7. Run `clog --setversion {version}`, verify changes were properly
    accounted for in `CHANGELOG.md`.
8. `git add CHANGELOG.md autopush/__init__.py` to add the two changes
    to the new release commit.
9. `git commit -m "chore: tag {version}"` to commit the new version and
    record of changes.
10. `git tag -s -m "chore: tag {version}" {version}` to create a signed
    tag of the current HEAD commit for release.
11. `git push --set-upstream origin release/{major}.{minor}` to push the
    commits to a new origin release branch.
12. `git push --tags origin release/{major}.{minor}` to push the tags to
    the release branch.
13. Submit a pull request on github to merge the release branch to
    master.
14. Go to the [autopush releases
    page](https://github.com/mozilla-services/autopush/releases), you
    should see the new tag with no release information under it.
15. Click the `Draft a new release` button.
16. Enter the tag for `Tag version`.
17. Copy/paste the changes from `CHANGELOG.md` into the release
    description omitting the top 2 lines (the a name HTML and the
    version) of the file.

    Keep these changes handy, you\'ll need them again shortly.

18. Once the release branch pull request is approved and merged, click
    `Publish Release`.
19. File a bug for stage deployment in Bugzilla, in the `Cloud Services`
    product, under the `Operations: Deployment Requests` component. It
    should be titled `Please deploy autopush {major}.{minor} to STAGE`
    and include the changes in the Description along with any additional
    instructions to operations regarding deployment changes and special
    test cases if needed for QA to verify.

At this point, QA will take-over, verify stage, and create a production
deployment Bugzilla ticket. QA will also schedule production deployment
for the release.
