# Developer workflow

This is a deliberately small, repository-verified workflow. The broader
[development](../development.md), [running](../running.md), and
[testing](../testing.md) pages contain additional options, but some historical
environment advice in those pages may lag the build files.

## Repository map

The Cargo workspace currently has eight members:

- `autoendpoint` — public publication and mobile-registration HTTP service;
- `autoconnect` — binary/server assembly for public WebSockets and the internal
  node listener;
- `autopush-common` — shared models, endpoint crypto, storage backends,
  reliability, metrics, logging, and utilities;
- `autoconnect-common` — protocol, registry, broadcast, and shared connection
  types;
- `autoconnect-settings` — configuration and process state;
- `autoconnect-web` — public and internal HTTP routes;
- `autoconnect-ws` — WebSocket transport loop and Ping manager;
- `autoconnect-ws-sm` — testable Hello/identified client state machine.

The most useful non-Rust paths are:

- `tests/integration/` — end-to-end Python tests and Docker Compose stack;
- `tests/load/` — Locust workload;
- `tests/notification/` — browser-oriented notification tests;
- `configs/` — current sample settings;
- `scripts/setup_bt.sh` — Bigtable emulator schema and GC policies;
- `docs/` — this mdBook.

## Tool versions

- The workspace's declared minimum Rust version is `1.94.0`.
- The Makefile and CI build with Rust `1.96`.
- Python tests declare Python `^3.12`.
- `make install_poetry` installs Poetry `2.0.0`; the integration-test image
  currently installs its own pinned Poetry version.
- Docker Compose v2 and the gcloud `cbt` component are needed for the full
  integration stack.

Use the build files rather than copying version numbers from this page when
they eventually change.

## Fastest end-to-end check

The integration image builds the Rust services, starts the Bigtable emulator,
creates all four families, and runs the Python tests:

```bash
make build-integration-test
make integration-test
```

Run a focused test locally through Poetry with:

```bash
make integration-test-local PYTEST_ARGS="-k test_basic_delivery"
```

That local target expects the services it addresses to have been built and
started separately; the Compose-based `integration-test` target is the more
self-contained path.

## Manual Bigtable emulator

For Rust tests or manual service runs:

```bash
gcloud beta emulators bigtable start --host-port=localhost:8086
```

In another shell:

```bash
export BIGTABLE_EMULATOR_HOST=localhost:8086
sh scripts/setup_bt.sh
```

The script waits for the emulator, creates `message`, `message_topic`,
`router`, and `reliability`, and applies the same future-cell-timestamp GC
pattern described in [Bigtable data model](bigtable.md).

For a lightweight Rust check after the emulator is ready:

```bash
cargo test --workspace --features=bigtable,emulator
```

The CI-like unit target additionally requires `cargo-nextest` and
`cargo-llvm-cov`:

```bash
make unit-test
```

## Running the two services manually

Start from `configs/autoconnect.toml.sample` and
`configs/autoendpoint.toml.sample`. The services must share the same Fernet
endpoint key, even though the setting names differ:

```bash
cp configs/autoconnect.toml.sample configs/autoconnect.local.toml
cp configs/autoendpoint.toml.sample configs/autoendpoint.local.toml
```

Uncomment and edit the local copies; do not put production credentials in
them. At minimum, configure the emulator DSN/table path and matching endpoint
keys. The two local files are examples for your checkout, not files supplied by
the repository.

- autoconnect: `crypto_key` / `AUTOCONNECT__CRYPTO_KEY`;
- autoendpoint: `crypto_keys` / `AUTOEND__CRYPTO_KEYS`.

They also need autoendpoint `auth_keys` for mobile registration-management
secrets. Generate compatible environment values with:

```bash
make gen-key
```

Then run each binary with Bigtable/emulator features and its own config file:

```bash
RUST_LOG=debug cargo run --bin autoconnect --features=bigtable,emulator -- \
  --config=configs/autoconnect.local.toml
```

```bash
RUST_LOG=debug cargo run --bin autoendpoint --features=bigtable,emulator -- \
  --config=configs/autoendpoint.local.toml
```

The defaults use separate public/internal autoconnect ports and an autoendpoint
port; copy exact values from the current sample files rather than assuming old
onboarding examples are still aligned.

## Build the documentation

The book uses mdBook plus `mdbook-mermaid`:

```bash
mdbook-mermaid install docs
mdbook build docs
```

The install step adds generated Mermaid assets that are not committed. After
it has run once, use the repository target for a local preview:

```bash
make doc-prev
```

## Best files for learning by execution

Start with these focused tests rather than a production log search:

- `test_basic_delivery`;
- `test_delivery_repeat_without_ack`;
- `test_repeat_delivery_with_disconnect_without_ack`;
- `test_topic_replacement_delivery`;
- `test_ttl_0_connected` / `test_ttl_0_not_connected`;
- `test_delete_saved_notification`;
- `test_msg_limit`.

Together they demonstrate the direct, recovery, stored, Topic, TTL-zero,
publisher-delete, and backlog-reset paths described in this chapter.
