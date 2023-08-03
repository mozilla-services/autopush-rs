#! /bin/bash
AWS_LOCAL_DYNAMODB=http://127.0.0.1:8000 \
BIGTABLE_EMULATOR_HOST=localhost:8086 \
CONNECTION_BINARY=autoconnect \
CONNECTION_SETTINGS_PREFIX="AUTOCONNECT__" \
DB_SETTINGS="bt_test.json" \
DB_DSN="grpc://localhost:8086" \
SKIP_SENTRY=1 \
CRYPTO_KEY="mqCGb8D-N7mqx6iWJov9wm70Us6kA9veeXdb8QUuzLQ=" \
venv/bin/py.test -svx $*
