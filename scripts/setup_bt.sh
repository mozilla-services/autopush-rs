#!/bin/sh -eu
# Usage: setup_bt.sh [PROJECT] [INSTANCE]
#
# Arguments:
#   [PROJECT]  (default: test)
#   [INSTANCE] (default: test)

PROJECT=${1:-"test"}
INSTANCE=${2:-"test"}

TABLE_NAME=${TABLE_NAME:-"autopush"}
MESSAGE_FAMILY=${MESSAGE_FAMILY:-"message"}
MESSAGE_TOPIC_FAMILY=${MESSAGE_TOPIC_FAMILY:-"message_topic"}
ROUTER_FAMILY=${ROUTER_FAMILY:-"router"}
RELIABILITY_FAMILY=${RELIABILITY_FAMILY:-"reliability"}

# The Bigtable emulator runs in a separate container. Compose's `depends_on`
# only guarantees that container has *started*, not that the emulator is
# accepting gRPC connections on its host/port yet. Without this wait the first
# `cbt` call races the emulator boot and intermittently fails with
# "connection refused", which (under `set -e`) aborts setup before pytest runs
# and leaves no junit report for CI to collect. Poll until the emulator answers.
WAIT_TIMEOUT=${WAIT_TIMEOUT:-60}
elapsed=0
until cbt -project "$PROJECT" -instance "$INSTANCE" ls >/dev/null 2>&1; do
  if [ "$elapsed" -ge "$WAIT_TIMEOUT" ]; then
    echo "Bigtable emulator not reachable after ${WAIT_TIMEOUT}s, giving up" >&2
    exit 1
  fi
  echo "Waiting for Bigtable emulator (${elapsed}s/${WAIT_TIMEOUT}s)..."
  sleep 1
  elapsed=$((elapsed + 1))
done

cbt -project $PROJECT -instance $INSTANCE createtable $TABLE_NAME
cbt -project $PROJECT -instance $INSTANCE createfamily $TABLE_NAME $MESSAGE_FAMILY
cbt -project $PROJECT -instance $INSTANCE createfamily $TABLE_NAME $MESSAGE_TOPIC_FAMILY
cbt -project $PROJECT -instance $INSTANCE createfamily $TABLE_NAME $ROUTER_FAMILY
cbt -project $PROJECT -instance $INSTANCE createfamily $TABLE_NAME $RELIABILITY_FAMILY

cbt -project $PROJECT -instance $INSTANCE setgcpolicy $TABLE_NAME $MESSAGE_FAMILY "maxage=1s or maxversions=1"
cbt -project $PROJECT -instance $INSTANCE setgcpolicy $TABLE_NAME $MESSAGE_TOPIC_FAMILY "maxage=1s or maxversions=1"
cbt -project $PROJECT -instance $INSTANCE setgcpolicy $TABLE_NAME $ROUTER_FAMILY maxversions=1
cbt -project $PROJECT -instance $INSTANCE setgcpolicy $TABLE_NAME $RELIABILITY_FAMILY "maxage=1s or maxversions=1"
