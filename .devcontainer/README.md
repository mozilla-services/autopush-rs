# Building autopush using a VSCode docker container.

## Prerequisites:

* [Docker](https://docs.docker.com/engine/install/)
* [VSCode](https://code.visualstudio.com/download)
* [git](https://git-scm.com/install/)
`
## Cautions:

1) `grpcio-sys v0.13.0+1.56.2-patched` does not build under Debian `trixie`, you must use `bookworm` until this is resolved.

## Launching and Configuring the container

In VSCode, you will need to re-open the project using the "Command Palette"[Control/Command]+[Shift]+[P] and enter: "Dev Containers: Reopen in Container". This will spawn a new docker instance containing the dev environment. Once the docker image has been built, you should be able to open a new terminal inside of that image.

Before you do your first `cargo build` you will need to install some local dependencies. 

```bash
sudo apt update;
sudo apt-get install build-essential libffi-dev libssl-dev pypy3-dev python3-virtualenv git glibc-source clang --assume-yes
```

> _*TODO*_: Need to figure out a way to include these into the docker image definition. added `make docker-init` for now.

Once that's done, you should be able to run 

```bash
cargo build
```

and have a successful build.

> _*NOTE*_: You will still need to install and run any locally required services like Bigtable emulators. See the Autopush "Development" guide in the Documentation tree. 

> _*NOTE*_: I am still trying to work out how to have ssh-agent access inside of the containerized environment. Be aware the `git commit` will probably fail inside of the container. You should instead commit work outside of the container (e.g. in your native, host environment).
