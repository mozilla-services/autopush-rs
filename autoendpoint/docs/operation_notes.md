# Operation Notes / Runbook (WIP)

## Logging

Logging level can be controlled via the `RUST_LOG` env variable on a per-module basis: <https://docs.rs/slog-envlogger/2.2.0/slog_envlogger/>.

Note that there is a compile-time ceiling (`STATIC_MAX_LEVEL`), which is fixed at build time by Cargo features on the log crate. No runtime setting, including `RUST_LOG`, can re-enable them.

Our own logging is unaffected by the compile-time ceiling, since application code logs through `slog`/`slog_scope` which reaches the logger directly. Any level our code emits (including `debug!`/`trace!`) is governed only by `RUST_LOG` at runtime.

**Dependency** logging is capped in release by `STATIC_MAX_LEVEL`. This prevents tracing events from `hyper`, `h2`, `tonic`, and `tower` from falling through to the log crate (if no tracing subscriber is installed).

If you need debug/trace from a log/tracing-emitting dependency (e.g. to debug an h2/tonic connection issue), a release binary won't produce it. You must either:

- run a debug build (ceiling is Debug; trace still compiled out), or
- build with a higher release ceiling by adjusting the log feature in the root Cargo.toml, e.g. `release_max_level_debug` or `release_max_level_trace` — then set `RUST_LOG` to the desired per-module level.

### Overflow buffer (`log_chan_size`)

Records are buffered to a background writer thread. When that buffer fills, records are dropped and each drop emits an ERROR-level "logger dropped messages due to channel overflow" report. If you see those reports, the process is logging faster than it can write — raise `AUTOEND__LOG_CHAN_SIZE` / `AUTOCONNECT__LOG_CHAN_SIZE` (default 20000; 0 means "use the default"), or reduce log verbosity.

## Settings

Nested settings (ex. `FcmSettings`) can be set with environment variables. For
example, to set `settings.fcm.ttl`, use `AUTOEND_FCM.TTL=60`
ent variables. For
example, to set `settings.fcm.ttl`, use `AUTOEND_FCM.TTL=60`
