#!/bin/bash
# Build and run the nginx-web image with chvm
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE_NAME="localhost/chvm-nginx-web:latest"
CHVM="$(command -v chvm)"

# Build only if Docker doesn't have the image
if ! docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
    echo "=== Building nginx-web image ==="
    docker build -t "$IMAGE_NAME" "$SCRIPT_DIR"
else
    echo "=== Image $IMAGE_NAME already exists in Docker ==="
fi

echo ""
echo "=== Running with chvm (port 8080 -> 80) ==="
doas "$CHVM" run -d --name chvm-nginx -p 8080:80 "$IMAGE_NAME"

echo ""
echo "Waiting for nginx to start..."
sleep 5

TAP_IP=$(doas "$CHVM" inspect chvm-nginx 2>/dev/null | grep '"ip_address"' | head -1 | sed 's/.*: *"\([^"]*\)".*/\1/' | cut -d/ -f1)

echo ""
echo "=== Testing ==="
echo "TAP IP: $TAP_IP"
echo ""
echo "--- curl http://$TAP_IP/ ---"
curl -s "http://$TAP_IP/" | head -5
echo "..."
echo ""
echo "--- curl http://$TAP_IP/health ---"
curl -s "http://$TAP_IP/health"
echo ""
echo ""
echo "To stop: doas chvm rm -f chvm-nginx"
