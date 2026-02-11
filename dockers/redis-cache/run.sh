#!/bin/bash
# Build and run the redis-cache image with chvm
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE_NAME="localhost/chvm-redis-cache:latest"
CHVM="$(command -v chvm)"

# Build only if Docker doesn't have the image
if ! docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
    echo "=== Building redis-cache image ==="
    docker build -t "$IMAGE_NAME" "$SCRIPT_DIR"
else
    echo "=== Image $IMAGE_NAME already exists in Docker ==="
fi

echo ""
echo "=== Running with chvm (port 16379 -> 6379) ==="
doas "$CHVM" run -d --name chvm-redis -p 16379:6379 "$IMAGE_NAME"

echo ""
echo "Waiting for redis to start..."
sleep 5

TAP_IP=$(doas "$CHVM" inspect chvm-redis 2>/dev/null | grep '"ip_address"' | head -1 | sed 's/.*: *"\([^"]*\)".*/\1/' | cut -d/ -f1)

echo ""
echo "=== Testing ==="
echo "TAP IP: $TAP_IP"
echo ""

if command -v redis-cli >/dev/null 2>&1; then
    echo "--- PING ---"
    redis-cli -h "$TAP_IP" PING

    echo ""
    echo "--- SET/GET ---"
    redis-cli -h "$TAP_IP" SET hello "world from chvm"
    redis-cli -h "$TAP_IP" GET hello

    echo ""
    echo "--- INFO (server section) ---"
    redis-cli -h "$TAP_IP" INFO server | head -10
else
    echo "(redis-cli not found on host, testing via exec)"
    doas "$CHVM" exec chvm-redis -- redis-cli PING
    doas "$CHVM" exec chvm-redis -- redis-cli SET hello "world from chvm"
    doas "$CHVM" exec chvm-redis -- redis-cli GET hello
fi

echo ""
echo "To stop: doas chvm rm -f chvm-redis"
