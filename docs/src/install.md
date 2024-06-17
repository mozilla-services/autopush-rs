# Installing

## System Requirements

Autopush requires the following to be installed. Since each system has
different methods and package names, it's best to search for each
package.

- Rust 1.66 (or later)

* build-essential (a meta package that includes):  
    -   autoconf
    -   automake
    -   gcc
    -   make

* (for integration testing) python3 and the python3 development (header files)

* libffi development

* openssl development

* python3 virtualenv

* git

For instance, if installing on a Fedora or RHEL-like Linux (e.g. an
Amazon EC2 instance):

``` bash
$ sudo yum install autoconf automake gcc make libffi-devel \
openssl-devel pypy pypy-devel python3-virtualenv git -y
```

Or a Debian based system (like Ubuntu):

``` bash
$ sudo apt-get install build-essential libffi-dev \
libssl-dev pypy-dev python3-virtualenv git --assume-yes
```

## Check-out the Autopush Repository

You should now be able to check-out the autopush repository.

``` bash
$ git clone https://github.com/mozilla-services/autopush-rs.git
```

Alternatively, if you're planning on submitting a patch/pull-request to
autopush then fork the repo and follow the **Github Workflow** documented
in [Mozilla Push Service - Code
Development](http://mozilla-push-service.readthedocs.io/en/latest/development/#code-development).

## Rust and Cargo

You can install Rust and Cargo (if not already present on your computer) by following the steps at [rustup.rs](https://rustup.rs), or by installing Rust from your systems package management system. Please note, that currently we require a minimum of rust 1.68.

You can find what version of rust you are running using

```bash
rustc --version
```

You can update to the latest version of rust by using

```bash
rustup update
```

You can build all applications by running

```bash
cargo build
```

## Scripts

After installation of autopush the following command line utilities are
available in the virtualenv `bin/` directory:

|                       |                                   |
|-----------------------|-----------------------------------|
| `autopush`            | Runs a Connection Node            |
| `autoendpoint`        | Runs an Endpoint Node             |
| `endpoint_diagnostic` | Runs Endpoint diagnostics         |
| `autokey`             | Endpoint encryption key generator |


If you are planning on using Google Cloud Bigtable, you will need to configure
your `GOOGLE_APPLICATION_CREDENTIALS`. See [How Application Default Credentials works](https://cloud.google.com/docs/authentication/application-default-credentials)

## Building Documentation

To build the documentation, you will need additional packages installed:

``` bash
cargo install mdbook
```

You can then build the documentation:

<!-- TODO: update to mdbook -->
``` bash
cd docs
make html
```

<a id="local_storage"> </a>

# Local Storage emulation

Local storage can be useful for development and testing. It is not advised to use emulated storage for any form of production environment, as there are strong restrictions on the emulators as well as no guarantee of data resiliance.

Specifying storage is done via two main environment variables / configuration settings.

**db_dsn**  
This specifies the URL to the storage system to use. See following sections for details.

**db_settings**  
This is a serialized JSON dictionary containing the storage specific settings.

## Using Google Bigtable Emulator locally

Google supplies [a Bigtable emulator](https://cloud.google.com/sdk/gcloud/reference/beta/emulators) as part of their free [SDK](https://cloud.google.com/sdk). Install the [Cloud CLI](https://cloud.google.com/sdk/docs/install), per their instructions, and then start the Bigtable emulator by running

```bash
gcloud beta emulators bigtable start
```

By default, the emulator is started on port 8086. When using the emulator, you will
need to set an environment variable that contains the address to use.

```bash
export BIGTABLE_EMULATOR_HOST=localhost:8086
```

Bigtable is memory only and does not maintain information between restarts. This
means that you will need to create the table, column families, and policies.

You can initialize these via the `setup_bt.sh` script which uses the `cbt`
command from the SDK:

```bash
scripts/setup_bt.sh
```

The `db_dsn` to access this data store with Autopendpoint would be:  
`grpc://localhost:8086`

The `db_setings` contains a JSON dictionary indicating the names of the message and router families, as well as the path to the table name.

For example, if we were to use the values from the initializion script above (remember to escape these values for whatever sysetm you are using):

```json
{"message_family":"message","message_topic_family":"message_topic","router_family":"router","table_name":"projects/test/instances/test/tables/autopush"}
```

## Using the "Dual" storage configuration (legacy)

Dual is a temporary system to be used to transition user data from one system to another. The "primary" system is read/write, while the "secondary" is read only, and is only read when a value is not found in the "primary" storage.

Dual's DSN Is `dual`. All connection information is stored in the `db_settings` parameter. (Remember to escape these values for whatever system you are using):

```json
{"primary":{"db_settings":"{\"message_family\":\"message\",\"router_family\":\"router\",\"table_name\":\"projects/test/instances/test/tables/autopush\"}","dsn":"grpc://localhost:8086"},"secondary":{"db_settings":"{\"message_table\":\"test_message\",\"router_table\":\"test_router\"}","dsn":"http://localhost:8000/"}}
```

## Configuring for Third Party Bridge services:

Working with mobile devices can present many challenges. One very significant one
deals with how mobile devices save battery very aggressively. Using your
mobile devices CPU and radio both require considerable battery power. This means
that maintaining something like a constant connection to a remote server, or
regularly "pinging" a server can cause your device to wake, spin up the CPU and
use the radio to connect to local wifi or cellular networks. This may cause your
application to be quickly flagged by the operating system and either aggressively
deactivated, or be flagged for removal.

Fortunately, the major mobile OS providers offer a way to send messages to devices
on their networks. These systems operate similarly to the way Push works, but
have their own special considerations. In addition, we want to make sure that
messages remain encrypted while passing through these systems. The benefit of
using these sorts of systems is that message delivery is effectively "free",
and apps that use these systems are not flagged for removal.

Setting up the client portion of these systems is outside the scope of this
document, however the providers of these networks have great documentation that
can help get you started.

As a bit of shorthand, we refer to these proprietary mobile messaging systems as
"bridge" systems, since they act as a metaphorical bridge between our servers and
our applications.

How we connect and use these systems is described in the following documents:

* [Apple Push Notification service (APNs)](apns.md)
* [Google's Fire Cloud Messaging service (FCM)](fcm.md)
