# Running Autopush

## Overview

To run Autopush, you will need to run at least one connection node, one
endpoint node, and a local storage system. The prior
section on Autopush architecture documented these components and their
relation to each other.

The recommended way to run the latest development or tagged Autopush
release is to use [docker](https://www.docker.com/). Autopush has
[docker](https://www.docker.com/) images built automatically for every
tagged release and when code is merged to master.

If you want to run the latest Autopush code from source then you should
follow the `developing` instructions.

The instructions below assume that you want to run Autopush with a local
Bigtable emulator for testing or local verification. The docker containers
can be run on separate hosts as well.

## Setup

*#TODO* rebuild the docker-compose.yaml files based off of syncstorage ones.

- [ ] rebuild docker-componse.yaml
  - [ ] initialize tables
- [] define steps here

<!-- The following block needs to be updated. The docker instructions are no
     longer correct. Please ignore.

<div style="text-decoration:line-through;display:none">
These instructions will yield a locally running Autopush setup with the
connection node listening on localhost port `8080`, with the endpoint
node listening on localhost port `8082`. Make sure these ports are
available on localhost before running, or change the configuration to
have the Autopush daemons use other ports.

1. Install [docker](https://www.docker.com/)

2. Install [docker-compose](https://docs.docker.com/compose/)

3. Create a directory for your docker and Autopush configuration:

    > ``` bash
    > $ mkdir autopush-config
    > $ cd autopush-config
    > ```

4. Fetch the latest `docker-compose.yml` file:

    > ``` bash
    > $ curl -O https://raw.githubusercontent.com/mozilla-services/autopush/master/docker-compose.yml
    > ```

> _**Note**_: The docker images used take approximately 1.5 GB of disk-space, make
sure you have appropriate free-space before proceeding.

</div>
-->

### Generate a Crypto-Key

As the `cryptography` section notes, you will need a `CRYPTO_KEY` to run
both of the Autopush daemons. To generate one with the docker image:

``` bash
$ docker run -t -i mozilla-services/autopush-rs autokey
CRYPTO_KEY="hkclU1V37Dnp-0DMF9HLe_40Nnr8kDTYVbo2yxuylzk="
```

Store the key for later use (including any trailing `=`).

## Start Autopush

Once you've completed the setup and have a crypto key, you can run a
local Autopush with a single command:

``` bash
$ CRYPTO_KEY="hkclU1V37Dnp-0DMF9HLe_40Nnr8kDTYVbo2yxuylzk=" docker-compose up
```

[docker-compose](https://docs.docker.com/compose/) will start up three
containers, two for each Autopush daemon, and a third for storage.

By default, the following services will be exposed:

`ws://localhost:8080/` - websocket server

`http://localhost:8082/` - HTTP Endpoint Server (See [the HTTP API](http.md))

You could set the `CRYPTO_KEY` as an environment variable if you are
using Docker. If you are running these programs "stand-alone" or outside
of docker-compose, you may setup a more thorough configuration using
config files as documented below.

> _**Note**_: The load-tester can be run against it or you can run Firefox with the
local Autopush per the `test-with-firefox` docs.

## Configuration

Autopush can be configured in three ways; by option flags, by
environment variables, and by configuration files. Autopush uses three
configuration files. These files use standard `ini` formatting similar to the following:

``` cfg
# A comment description
;a_disabled_option
;another_disabled_option=default_value
option=value
```

Options can either have values or act as boolean flags. If the option is
a flag it is either True if enabled, or False if disabled. The
configuration files are usually richly commented, and you're encouraged
to read them to learn how to set up your installation of autopush.

*Note*: any line that does not begin with a `\#` or `;` is
considered an option line. if an unexpected option is present in a
configuration file, the application will fail to start.

Configuration files can be located in:

* in the /etc/ directory
* in the configs subdirectory
* in the $HOME or current directory (prefixed by a period '.')

The three configuration files are:

* *autopush_connection.ini* - contains options for use by the
    websocket handler. This file's path can be specified by the
    `--config-connection` option.
* *autopush_shared.ini* - contains options shared between the
    connection and endpoint handler. This file's path can be specified
    by the `--config-shared` option.
* *autopush_endpoint.ini* - contains options for the HTTP handlers
    This file's path can be specified by the `--config-endpoint` option.

### Sample Configurations

Three sample configurations, a base config, and a config for each
Autopush daemon can be found at
<https://github.com/mozilla-services/autopush/tree/master/config>

These can be downloaded and modified as desired.

### Config Files with Docker

To use a configuration file with [docker](https://www.docker.com/),
ensure the config files are accessible to the user running
[docker-compose](https://docs.docker.com/compose/). Then you will need
to update the `docker-compose.yml` to use the config files and make them
available to the appropriate docker containers.

Mounting a config file to be available in a docker container is fairly
simple, for instance, to mount a local file `autopush_connection.ini`
into a container as `/etc/autopush_connection.ini`, update the
`autopush` section of the `docker-compose.yml` to be:

``` yaml
volumes:
  - ./boto-compose.cfg:/etc/boto.cfg:ro
  - ./autopush_connection.ini:/etc/autopush_connection.ini
```

Autopush automatically searches for a configuration file at this
location so nothing else is needed.

*Note*: The `docker-compose.yml` file
provides a number of overrides as environment variables, such as `CRYPTO_KEY`. If these values are not defined,
they are submitted as `""`, which will
prevent values from being read from the config files. In the case of
`CRYPTO_KEY`, a new, random key is
automatically generated, which will result in existing endpoints no
longer being valid. It is recommended that for docker based images, that
you **\*always**\* supply a `CRYPTO_KEY` as
part of the run command.

### Notes on GCM/FCM support

*Note*: GCM is no longer supported by Google. Some legacy users can
still use GCM, but it is strongly recommended that applications use FCM.

Autopush is capable of routing messages over Firebase Cloud Messaging
for android devices. You will need to set up a valid
[FCM](https://firebase.google.com/docs/cloud-messaging/) account. Once
you have an account open the Google Developer Console:

* create a new project. Record the Project Number as "SENDER_ID". You
    will need this value for your android application.

* in the `.autopush_endpoint` server config file:
    - add `fcm_enabled` to enable FCM routing.
    - add `fcm_creds`. This is a json block with the following format:

    ```{"APP ID": {"projectid": "PROJECT ID NAME", "auth":"PATH TO PRIVATE KEY FILE"}, ...}```

    (_see [Configuring for the Google GCM/FCM](fcm.md) for more details_)

where:

**app_id**: the URL identifier to be used when registering endpoints.
(e.g. if "reference_test" is chosen here, registration requests should
go to `https://updates.push.services.mozilla.com/v1/fcm/reference_test/registration`)

**project id name**: the name of the **Project ID** as specified on the
<https://console.firebase.google.com/> Project Settings \> General page.

**path to Private Key File**: path to the Private Key file provided by
the Settings \> Service accounts \> Firebase Admin SDK page. *NOTE*:
This is **\*NOT**\* the "google-services.json" config file.

Additional notes on using the FCM bridge are available [on the
wiki](https://github.com/mozilla-services/autopush/wiki/Bridging-Via-GCM).
