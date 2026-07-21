# Release Process {#releasing}

<div style="color:red;border:2px solid red;padding:5px; margin:10px 5px ">
<b><i>NOTE</i></b> This page is outdated, however the "Release Steps" are still a useful checklist.
</div>

Autopush has a regular 2-3 week release to production depending on
developer and QA availability. The developer creating a release should
handle all aspects of the following process as they\'re done closely in
order and time.

## Versions

Autopush uses a `{major}.{minor}.{patch}` version scheme (see [semver.org](https://semver.org/) for more info). New `{major}` versions are only issued if backwards compatibility is affected.
Minor versions are used to indicate new backwards compatible features. Patch
versions are for backwards-compatible bug fixes.

## Dev Releases

When changes are committed to the `main` branch, a github action will build and deploy the code automatically. The new image is pushed to Google Artifact Registry with the "latest" tag,
which is automatically synced in autopush dev namespace by ArgoCD.

The development environment can be verified at its endpoint/wss
endpoints:

- Websocket: <wss://autoconnect.dev.mozaws.net/>
- Endpoint: <https://updates-autopush.dev.mozaws.net/>

## Stage/Production Releases

### Pre-Requisites

To create a release, you will need appropriate access to the autopush
GitHub repository with push permission.

You will also need [git-cliff](https://git-cliff.org/docs/installation/)
installed to create the `CHANGELOG.md` update.

### Release Steps

In these steps, the `{version}` refers to the full version of the
release.

i.e. If a new minor version is being released after `1.21.0`, the
`{version}` would be `1.22.0`.

1. Switch to the `main` branch of autopush.
2. `git pull` to ensure the local copy is completely up-to-date.
3. `git diff origin/main` to ensure there are no local staged or
   uncommited changes.
4. Create the release branch from `main`:
   `git checkout -b release/{version}`.

   The branch name includes the full `{major}.{minor}.{patch}` version,
   so every release (major, minor, or patch) gets its own branch off the
   current `main`.

5. Edit `Cargo.toml` `version` key so that the version number reflects the
   desired release version.
6. Regenerate the Cargo lockfile by running `cargo generate-lockfile`,
   and ensure there are no unwanted changes.
7. Update `CHANGELOG.md` by running `git cliff -u`.
8. `git add CHANGELOG.md Cargo*` to add the changes
   to the new release commit.
9. `git commit -m "chore: tag {version}"` to commit the new version and
   record of changes.
10. `git push --set-upstream origin release/{version}` to push the
    commits to a new origin release branch.
11. Submit a pull request on github to merge the release branch to
    main.
12. Once merged, `git checkout main && git pull` to fast-forward to current head.
13. Get the merge commit SHA, either from the PR page ("merged commit `abc1234` into main")
    or programmatically with `gh pr view <pr-number> --json mergeCommit --jq '.mergeCommit.oid'`
14. Run `git tag -s -m "chore: tag {version}" {version} <merge-commit-sha>` to tag the commit that actually landed on `main`.
15. Push the tag with `git push origin {version}`; this is what triggers the deployment in CircleCI.
16. Go to the [autopush releases
    page](https://github.com/mozilla-services/autopush-rs/releases), you
    should see the new tag with no release information under it.
17. Click the `Draft a new release` button.
18. Enter the tag for `Tag version`.
19. Copy/paste the changes from `CHANGELOG.md` into the release
    description omitting the top 2 lines (the a name HTML and the
    version) of the file.
20. Staging is auto-synced. Verify that the deployment is successful by testing the staging endpoints:
    - Websocket: <wss://autoconnect.stage.mozaws.net/>
    - Endpoint: <https://updates-autopush.stage.mozaws.net/>
21. Once staging is verified, you may release to production. See [internal documentation](https://mozilla-hub.atlassian.net/wiki/spaces/CLOUDSERVICES/pages/1025736721/Push+service+developer+documentation#Releasing-to-Production%3A) for production release process.
