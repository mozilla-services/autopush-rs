# Documentation Notes and comments

## mdbook

To build the documentation, install mdbook

```bash
cargo install mdbook mdbook-mermaid
```

then run

```bash
mdbook build
```

This will generate the html files into the `./output` directory. You can also run

```bash
mdbook serve
```

which will serve those files on `http://localhost:3000`

### Integration with rustdoc

`mdbook` does not cleanly integrate with `rustdoc` at this time. It's possible (via some fun github actions) to build the docs and include them in the deploy

## Building Pages using Github Actions

### Setup

Github Actions allows for various CI like steps to run. Currently, there is [publish_docs.yml](../.github/workflows/publish_docs.yml).
It currently has two "jobs", one to do the build, another to deploy the built artifact to Github pages.

Under the repo settings, be sure to set:

* Actions
  * General
    _Actions permissions_
    ◉**Allow $USER, and select non-$USER, actions and reusable workflows**
    ☑ Allow actions created by GitHub
    ☑ Allow actions by Marketplace verified creators
    _Artifact and log retention_
    (can use default)
    _Fork pull request workflows from outside collaborators_
    ◉ **Require approval for first-time contributors**
    _Workflow permission_
    ◉ Read and write permissions
    ☑ Allow GitHub Actions to create and approve pull requests
  * Runners
    No settings needed

* Pages
  **Build and deployment**
  Source: GitHub Actions

### Running

You specify triggers within the `.github/workflows` document. Currently `publish_docs.yml` 
