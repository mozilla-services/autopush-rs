# Operation Notes / Runbook (WIP)
- Logging level can be controlled via the `RUST_LOG` env variable on a per-module
  basis: https://docs.rs/slog-envlogger/2.2.0/slog_envlogger/
- Nested settings (ex. `FcmSettings`) can be set with environment variables. For
  example, to set `settings.fcm.ttl`, use `AUTOEND_FCM.TTL=60`