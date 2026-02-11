#!/bin/bash
# Build and run the alpine-hello image with chvm
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE_NAME="localhost/chvm-alpine-hello:latest"
CHVM="$(command -v chvm)"

# Build only if Docker doesn't have the image
if ! docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
    echo "=== Building alpine-hello image ==="
    docker build -t "$IMAGE_NAME" "$SCRIPT_DIR"
else
    echo "=== Image $IMAGE_NAME already exists in Docker ==="
fi

echo ""
echo "=== Running with chvm ==="
doas "$CHVM" run --rm "$IMAGE_NAME"
