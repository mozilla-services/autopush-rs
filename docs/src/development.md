# Configuring a Stand Alone Development Environment

The following is a brief overview of the steps required to create a "stand alone" environment.

## Pre-requirements

This tutorial presumes the following items are installed on your system.

- [Docker CLI](https://docs.docker.com/engine/install/)
- [Google Bigtable emulator](https://docs.cloud.google.com/bigtable/docs/emulator) (Note that this also requires docker)
- [Redis](https://redis.io/docs/latest/operate/oss_and_stack/install/install-stack/docker/) (this is semi-optional, but used by the Reliability service)

The `grpcio_sys` library will not compile under Debian Trixie (Linux kernel 1.6+) / Ubuntu 25.10+

If you are running [VSCode](https://code.visualstudio.com/download), you can take advantage of the "Containerized Development" option. See the `README.md` file in the `.devcontainer` directory for details. If not, you may be able to work inside of a Docker image using the following script:

```bash
#!/bin/bash
# This script will start an interactive shell for autopush development in Debian `bookworm`
# Note that Debian `trixie` breaks a dependency that will cause `grpcio_sys` to fail to compile.
# If you wish to use a different shell you can define it as an environment variable before you
# call this script (e.g. `DSHELL=sh ./docker-shell`) be aware that not all shells are included
# in the default IMAGE.
export DEFAULT=rust:1.91-bookworm
# this uses a bash "default argument" syntax. If there's no value for $1
# (the first bash argument), it will use whatever DEFAULT is set to.
export SHELL=${DSHELL:-/bin/bash}
export IMAGE=${1:-$DEFAULT}
shift
#PORTS=-p8000:8000
export PORTS=-p9160:9160
docker run $OPTS $PORTS \
    --net=host \
    --workdir=/src \
    -v $HOME/.rustup:/app/.rustup \
    -v $HOME/.cargo:/app/.cargo \
    -v `pwd`:/src \
    --entrypoint $SHELL -it $IMAGE $@
```

The emulator and other locally hosted support libraries should still be accessible inside and outside of the docker image.

### Launching the Bigtable Emulator

Bigtable can be launched and configured using the following script. This script should _not_ be run from inside of the `bookworm docker` described above.

```bash
#!/bin/bash
# First, set the handle limit very high.
ulimit -n 64000
gcloud beta emulators bigtable start --host-port=localhost:8086 &
# Wait for bigtable to finish initializing
sleep 15
export BIGTABLE_EMULATOR_HOST=localhost:8086
# initialize the tables we'll use.
cbt -project test -instance test createtable autopush && \
cbt -project test -instance test createfamily autopush message && \
cbt -project test -instance test createfamily autopush message_topic && \
cbt -project test -instance test createfamily autopush router && \
cbt -project test -instance test createfamily autopush reliability && \
cbt -project test -instance test setgcpolicy autopush message maxage=1s && \
cbt -project test -instance test setgcpolicy autopush router maxversions=1 && \
cbt -project test -instance test setgcpolicy autopush message_topic maxversions=1 and maxage=1s && \
cbt -project test -instance test setgcpolicy autopush reliability maxversions=1 and maxage=6d
```

### Launching Redis

Redis can be started using the following script. This script should _not_ be run from inside of the `bookworm docker` described above.

```bash
#! /bin/bash
# Save every 60 seconds / 1000 key changes.
# retain timestamp (by default) for 1 hour
# expose ports for redis (6379) and insight (8001) (if available)
docker run \
    --rm \
    -e REDIS_ARGS="--save 60 1000" \
    -e REDISTIMESERIES_ARGS="RETENTION_POLICY=3600000" \
    --name redis-stack \
    -p 6379:6379 \
    -p 8001:8001 \
    -v`pwd`/r_data:/data redis/redis-stack-server:latest
```

## Creating autopush configuration files

Autopush can be configured either by environment variables, parameter options, or by defining a configuration file.
The configuration file is probably the easiest to work with. It is probably easiest to store these under `configs` using a descriptive format like `$app_$storage.toml`. For example, a sample configuration file for `autoendpoint` using `bigtable` would be

`autoendpoint_bigtable.toml`

```toml
crypto_keys = "[YourKeyHere===]"
# This is the DSN to the bigtable emulator you created prior.
db_dsn = "grpc://localhost:8086"
db_settings = "{\"message_family\":\"message\",\"router_family\":\"router\", \"table_name\":\"projects/test/instances/test/tables/autopush\"}"
# This is the address of the host. It's used internally for reporting and connections. If you plan on connecting from a remote
# machine on your local network, make sure to specify your local IP address.
host = "127.0.0.1"
# Use human friendly reporting.
human_logs = 1
port = 9160
reliability_dsn = "redis://localhost:6379"
# include test tracking keys (second is from load tester)
tracking_keys = "[\"Tracking_Key_Signature\",\"Tracking_Key_Signature\"]"

[fcm]
credentials = ... # A JSON dictionary containing the Android mobile FCM credentials.

[apns]
channels = ... # A JSON dictionary containing the iOS APNS mobile Credentials.
```

> _*NOTE*_: If you are running a containerized development environment, you should change the `localhost` references to point to your host's internal network address. This is usually the `.1` address of whatever subnet Docker has selected for these instances. (`ip route | grep "default" | cut -f3 -d' '` may work. You also may wish to use environment flags like `INDOCKER` to indicate to scripts or applications that they are running inside of a Docker instance and should look for the Host IP.)

## Starting the Autopush executables

As you may have noticed, I like to use bash scripts for things. This includes starting a local copy of autoconnect and autoendpoint.

I use

````bash
#!/bin/bash
# Start an Autopush executable with logging
# The first argument (if specified) is the executable to run
# The second argument (if specified) is the data storage type to use.
# If you wish to use a different logging level than `trace`, use
# the `LOG_LEVEL` environment variable. (e.g.
#
#   ```
#   LOG_LEVEL=info bash run.bash autoendpoint
#   ```
#
# This script presumes that you have configuration files declared in
# a `config` subdirectory that match the `application`_`data storage`.toml naming
# convention.
#
ulimit -s 64000
APP=${1:-autoconnect}
DB=${2:-bigtable}
if [ "$DB" != "dynamodb" ]; then
    FEATURES="--features=$DB --features=emulator"
else
    FEATURES="--features=$DB"
fi
#
LOG_LEVEL=${LOG_LEVEL:-trace}
RUST_LOG=autopush=$LOG_LEVEL,autopush_common=$LOG_LEVEL,autoendpoint=$LOG_LEVEL,autoconnect=$LOG_LEVEL,slog_mozlog_json=info,warn
#
# For sanity sake, display the command we're trying to run.
echo RUST_LOG=$RUST_LOG cargo run --bin $APP $FEATURES $XTRA -- --config=configs/$APP\_$DB.toml
RUST_LOG=$RUST_LOG cargo run --bin $APP $FEATURES $XTRA -- --config=configs/$APP\_$DB.toml
````

For example:

```bash
run.bash autoconnect
```

will start a copy of the autoconnect websocket server using the configuration specified in `./configs/autoconnect_bigtable.toml`

```bash
run.bash autoendpoint
```

will start a copy of the autoendpoint REST server using the configuration specified in `./configs/autoendpoint_bigtable.toml`

## Container Restrictions and Limitations

Your docker instance will probably _not_ have full access to git. Read operations will be possible, but write operations will probably fail due to missing credentials. If you wish, you can always add your credentials inside of the Docker image, or just call the git functions from the Host. 

## Testing Inside of a Container

### Unit Tests
Since the container is a smaller, more restricted environment, it may be preferable to run Unit Tests outside of the container. `cargo` will "helpfully" compile the source for the current environment, which may be a problem in some situations. Fortunately, autopush uses `nextest` which can write and read from archives. 

If you wish to use Host testing:

1) inside the development container execute `cargo nextest archive --archive-file autopush.tar.zst` (The archive file name can be different.)
2) Once the archive file has been created, from the source top directory on the Host system `cargo nextest run --archive-file autopush.tar.zst --workspace-remap .`

### Integration Tests

Because the integration tests use python, there is less likelihood of code recompilation. The Host system should be able to see the executables in the `./target/debug` directory, and those _should_ be able to run on the Host system.
