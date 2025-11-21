#!/usr/bin/env bash
set -euo pipefail

# Minimal smoke test for autoconnect container
#
# This script verifies that a built autoconnect Docker image can start successfully
# and respond to health checks. The container is configured with minimal settings
# sufficient to start the server and respond to the __lbheartbeat__ endpoint.
#
# Usage: ./autoconnect.sh [image_name]

readonly IMAGE_NAME="${1:-autoconnect}"
readonly PORT=8080
readonly MAX_RETRIES=30
readonly RETRY_DELAY=1

CONTAINER_ID=""

# Cleanup function to ensure container is removed on exit
cleanup() {
  if [[ -n "$CONTAINER_ID" ]]; then
    echo "Cleaning up container..."
    docker stop "$CONTAINER_ID" >/dev/null 2>&1 || true
    docker rm "$CONTAINER_ID" >/dev/null 2>&1 || true
  fi
}

trap cleanup EXIT

echo "Starting autoconnect container from image: $IMAGE_NAME"

CONTAINER_ID=$(docker run --detach --quiet \
  -e AUTOCONNECT__PORT=$PORT \
  -e AUTOCONNECT__ROUTER_PORT=8081 \
  -e AUTOCONNECT__ENDPOINT_PORT=8082 \
  -e AUTOCONNECT__DB_DSN=grpc://localhost:8086 \
  -e AUTOCONNECT__DB_SETTINGS='{"table_name":"projects/test/instances/test/tables/autopush"}' \
  -p "$PORT:$PORT" \
  "$IMAGE_NAME")

echo "Container started!"

# Wait for container to be ready with retry logic
echo "Waiting for health check endpoint to respond..."
for i in $(seq 1 "$MAX_RETRIES"); do
  if curl --fail --silent --show-error "http://localhost:$PORT/__lbheartbeat__" >/dev/null 2>&1; then
    echo "Health check passed after $i attempt(s)!"
    exit 0
  fi
  sleep "$RETRY_DELAY"
done

echo "Health check failed after $MAX_RETRIES attempts!"
echo ""
echo "Container logs:"
docker logs "$CONTAINER_ID"
exit 1
