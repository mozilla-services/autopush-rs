# Data Storage Systems

## Introduction

*PLEASE NOTE* Autopush only used Bigtable in production. Any other datastore is considered experimental and has only been used in small, test environments. Please use caution if you decide to use an alternative data storage system. As always, your improvements and suggestions are welcome.

Autopush has had a number of data storage systems. Autopush is generally designed around schemaless storage, since we often require rapid read/write access, and schemaless tends to provide that at the best cost for the scale we tend to operate at. Originally, Autopush used AWS DynamoDB, and was optimized heavily toward that architecture. Then, for many non-technical reasons, we were forced to migrate to GCP Bigtable. At that time, we took the opportunity to re-design the storage system to be a bit more friendly toward more generic data storage systems. In essence, so long as the data store complies with the trait outlined in `autopush_common::db::client::DbClient`, Autopush really doesn't care what it's talking to. You, however, probably do. Each data storage system has it's own unique quirks and constraints, and you should be mindful of these when selecting or designing new storage systems.

Autopush has two main "tables" it refers to. (These may not be distinct tables in the storage, but should be considered semi-discrete collections of related data.) One is the **router** table. This table contains information keyed off of the UAID, and refers to the known channels for that UAID, as well as information about how to deliver messages to that UAID (e.g. it can contain information about mobile routing, or which internal node a user may be currently connected to, or other relevant info). The second is the **message** table, which contains the encrypted message content. This content is unreadable since the cryptographic keys are only stored in the client or with the original publisher.

It's important to note that the data stored in these "tables" needs to be regularly swept and "expired" data should be removed. Each data store system has it's own way of doing this, but untended data will grow remarkably fast which will impact service costs. Since each data store has it's own method of garbage collection, it's best to refer to the code to see how it should be handled.

## Storage Options

Please note: configuration options will presume that you are using configuration files. Remember, all configuration options can be specified using environment variables. See the sample configuration options for details.

From what we've observed, the general usage pattern for our system tends to be that a large number of users create a subscription, then those subscriptions go "idle". The bulk of our messages go into storage and are never retrieved. Internally, we consider this the "long storagegit la" problem.

As an additional "bonus", Publishers frequently ignore our 404/410 responses and continue to publish to subscription URLs that are no longer valid. These invalid subscriptions still require a DB lookup, so you may wish to also add rules to temporarily block IPs that have too many 404/410 responses.

### Bigtable

This is the production storage engine that is used by Mozilla. To use it, compile the autopush code with `--features=bigtable`.

It offers reasonable cost for the sort of scale that we see, and the performance of the data system is very good. While probably not advisable for any form of reliable production work, you can experiment with [using the Bigtable emulator](bigtable-emulation.md) fairly easily.

#### Configuration
The Bigtable DSN begins with the prefix `grpc` and points to the host and port of the Bigtable database (e.g. to point to a local emulator you would specify `grpc://localhost:8086`)

Bigtable's database configuration options are stored in the `db_settings` as a serialized JSON string. The settings include:
* `message_family` the Bigtable cell family that refers to the **message** data collections
* `router_family` the Bigtable cell family that refers to the **router** data collections
* `table_name` the Bigtable internal path URI to the table. You construct this by using the following template `projects/{YOUR PROJECT}/instances/{INSTANCE NAME}/tables/{TABLE NAME}`. For example, if I had created a GCP project named "test-prod", that has an instance ID of "test-prod-us" which contained a table named "auto-test", the `table_name` would be `projects/test-prod/instances/test-prod-us/tables/auto-test`.
The `scripts/setup_bt.sh` file contains a collection of commands that can be used to configure Bigtable storage, or at least aid in setting up the table.

### Redis/Valkey

Redis is a memory based key/value storage system. Valkey is an Open Source, API equivalent, "drop-in" replacement for Redis. For simplicity, I will refer to both systems as "Redis". To use it, compile the autopush code with `--features=redis`.

Redis is very responsive, but also quite memory intensive. While initial load tests have shown surprising results, it should be noted that there is a drastic difference between the load test environment and reality. It can be prohibitively expensive to replicate our traffic in a test environment, so our testing system focuses on a more aggressive use model (tests use a high rate of short term message delivery and retrieval) at the expense of productions "long storage" stress (where messages may sit in storage for weeks, or are potentially abandoned by no longer active User Agents).

That said, for orgs of around 1,000 users, or for personal use, this is a fine option that requires very little setup or maintenance.

#### Configuration

The Redis DSN begins with the prefix `redis` and points to the host and port of the server (e.g. `redis://localhost:6739`)

Redis' database configuration options are stored in `db_settings` as a serialized JSON string. These are optional and default values will be used.
The settings include:
* `create_timeout`: number of seconds to wait to create a new connection to Redis
* `router_ttl`: the number of seconds for a router record to live. The default is `MAX_ROUTER_TTL`, or `60 * 86,400` (60 days). Whenever the UAID connects, this TTL is reset. Only records for UAIDs that have not connected in 60 days are dropped.
* `notification_ttl`: the maximum number of seconds for a message to live. Messages live by their
provided `TTL` header. Messages that specify a `TTL` greater than this number will be set to this limit. By default, the `MAX_NOTIFICATION_TTL_SECS` value of `30 * 86,400` (30 days). This feature is offered here because of the in-memory nature of Redis, and concern that having too many records may impact Redis' overall ability to function on smaller systems. Your experiences and observations will help others learn how to best optimize this value or determine if it's required for other data storage systems.

Redis does not require any prep work before using.

### PostgreSQL

PostgreSQL is a SQL storage engine which has several features that allow it to perform similar to a schemaless engine. While it is not as performant as some schemaless systems, it is also a fairly common data storage system which may be more familiar to some. To use it, compile the autopush code with `--features=postgres`.


#### Configuration

The PostgreSQL DSN begins with the prefix `postgres` and can contain the login information and schema (e.g. `postgres://dbuser:hunter2@localhost:5432/autopush`)

PostgreSQL's database configuration options are stored in `db_settings` as a serialized JSON strong.
These are optional and default values will be used.
The settings include:
* `schema`: The name of the PostgreSQL schema to use (default `public`)
* `router_table`: the name of the **router** table (default `router`)
* `message_table`: the name of the **message** table (default `message`)
* `meta_table`: the name of the table to store additional routing information for the user (default `meta`)
* `reliability_table`: the name of the table to store reliability tracking information (default `reliability`) (Note, the `reliable_report` feature, which uses this table, **REQUIRES** Redis/Valkey to be enabled.)
* `max_router_ttl`: the number of seconds for a router record to live. The default is `MAX_ROUTER_TTL`, or `60 * 86,400` (60 days). Whenever the UAID connects, this TTL is reset. Only records for UAIDs that have not connected in 60 days are dropped.

There is a schema declaration file located in `autopush-common/src/db/postgres/schema.psql` which sets up an initial schema for PostgreSQL. Note that this file may change in the future and that migration files may be published separately. Please note any changes appearing in the `CHANGELOG.md` file when updating.
