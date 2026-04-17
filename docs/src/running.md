# Running Autopush

## Overview

To run Autopush, you will need to run at least one connection node, one
endpoint node, and a local storage system. The prior
section on Autopush architecture documented these components and their
relation to each other. If you plan on running Autopush in a production environment,
you will also need to set up TLS termination. This can be done using either a reverse
proxy (like nginx) or by using a load balancer system from your provider. This document
will not go into details on how to create this system. Your TLS termination system
should be configured to point Websocket traffic to the connection node point (default
port 8080) and HTTP traffic to the endpoint node point (default port 8082). Note that
some proxy systems require adding an `UPGRADE` header to the Websocket traffic, so you
may need to add that to your configuration.

When constructing your firewall rules, remember that autoconnect and autendpoint will
need to be able to route messages using the `router_port` (default 8081). This port
should NOT be public, but should be accessible to all nodes in the cluster.

While there are some docker files present, these are mostly used for testing
and are not intended for production use. The instructions below will allow you to run
Autopush "locally" in a Ubuntu / Debian environment. If you prefer, the
[development](development.md) document has more details. This may be addressed at a
later date.

If you want to run the latest Autopush code from source then you should
follow the [Installing](install.md) instructions.

The instructions below assume that you want to run Autopush with a local
Bigtable emulator for testing or local verification.

### Generate a Crypto-Key

As the `cryptography` section notes, you will need a `CRYPTO_KEY` to run
both of the Autopush daemons. To generate one with the docker image:

``` bash
$ make gen-key
[...]
# Crypto Keys:
AUTOCONNECT__CRYPTO_KEYS="[_4H...yQ=]"
AUTOEND__CRYPTO_KEYS="[_4H...yQ=]"


# Auth Key:
AUTOEND__AUTH_KEYS=["63-...8A="]

```

Store the key for later use (including any trailing `=`). By default autopush will look for this value as an environment variable, so you may want to use something like [direnv](https://direnv.net/) and add

```
export AUTOCONNECT__CRYPTO_KEYS="[Your-Key-Here00000000000000000000000000000==]"
export AUTOEND__CRYPTO_KEYS="[Your-Key-Here00000000000000000000000000000==]"
```

## Starting optional emulators

If you are planning on doing local development work with Autopush, you may wish to run emulators for
[Bigtable](https://docs.cloud.google.com/bigtable/docs/emulator) and [Redis](https://redis.io/docs/latest/operate/oss_and_stack/install/install-stack/docker/). This document does not detail the steps required, because they may change.

Once you have the Bigtable emulator installed, you may with to use the following bash script to start it. (Note, this presumes that you are using the GCLI interface. Please edit as needed.)

```bash
#!/bin/bash
# Set the file limit higher than the standard default
ulimit -n 64000
# If you're running a docker instance, you may want to use the following line
#docker run -p 127.0.0.1:8086:8086 --rm -ti google/cloud-sdk gcloud beta emulators bigtable start --host-port=0.0.0.0:8086 &
# Start the GCLI instance
gcloud beta emulators bigtable start --host-port=localhost:8086 &
# Wait for the emulator to start (YMMV)
sleep 15
# Export this so that the code knows where to look for the emulator
export BIGTABLE_EMULATOR_HOST=localhost:8086
# Finally, execute the following to create the tables and families that autopush will need.
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

Redis is used by the Push Reliability functions. For Redis, I tend to use the following script:
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
## Configuration

Autopush can be configured in two ways; by
environment variables, and by configuration files. See [the configuration documentation](config_options.md) for more details on the available options and how to set them. Each application has it's own configuration file, specified using the `--config` option. This option specifies the path of the TOML configuration file to use.

## Start Autopush

The following instructions will allow you to run Autopush "locally" in a Ubuntu / Debian environment. If you prefer, the [development](development.md) document has more details.

One pattern that has helped with development and local testing is to create configuration files that match the application and data store. For example, you might have a configuration for `autoconnect_bigtable.toml` which contains configuration information for the connect node using bigtable as it's data store. (You would also want to use
`autoendpoint_bigtable.toml` in this case since data stores need to be shared between the applications.)

This allows you to run an easier command like:

For `run.bash`:
```bash
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
# First, increase the file limit for the process, which is required for the Bigtable emulator.
ulimit -s 64000
# Set the application from the first argument, or default to `autoconnect`
APP=${1:-autoconnect}
# Set the data store from the second argument, or default to `bigtable`
DB=${2:-bigtable}
FEATURES="--features=$DB"
# Set the logging level for autopush and related crates. The default is `trace`,
# but you can set it to whatever you like using the `LOG_LEVEL` environment variable.
LOG_LEVEL=${LOG_LEVEL:-trace}
RUST_LOG=autopush=$LOG_LEVEL,autopush_common=$LOG_LEVEL,autoendpoint=$LOG_LEVEL,autoconnect=$LOG_LEVEL,slog_mozlog_json=info,warn
#
# For sanity sake, display the command we're trying to run.
echo RUST_LOG=$RUST_LOG target/debug/$APP --config=configs/$APP\_$DB.toml
RUST_LOG=$RUST_LOG target/debug/$APP --config=configs/$APP\_$DB.toml
```

To start autoendpoint with bigtable as the data store, you would run:
```bash
bash run.bash autoendpoint
```
(Remember `bigtable` is the default data store, so you don't need to specify it.)

To start autoconnect with the same data store, you would run:
```bash
bash run.bash
```
(remember both `autoconnect` and `bigtable` are the default values, so you don't need to specify either of them.)
