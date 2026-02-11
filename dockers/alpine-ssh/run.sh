#!/bin/bash
# Build and run the alpine-ssh image with chvm
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE_NAME="localhost/chvm-alpine-ssh:latest"
CHVM="$(command -v chvm)"

# Build only if Docker doesn't have the image
if ! docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
    echo "=== Building alpine-ssh image ==="
    docker build -t "$IMAGE_NAME" "$SCRIPT_DIR"
else
    echo "=== Image $IMAGE_NAME already exists in Docker ==="
fi

echo ""
echo "=== Running with chvm (port 2222 -> 22) ==="
doas "$CHVM" run -d --name chvm-ssh -p 2222:22 "$IMAGE_NAME"

echo ""
echo "Waiting for sshd to start..."
sleep 5

TAP_IP=$(doas "$CHVM" inspect chvm-ssh 2>/dev/null | grep '"ip_address"' | head -1 | sed 's/.*: *"\([^"]*\)".*/\1/' | cut -d/ -f1)

echo ""
echo "=== Testing ==="
echo "TAP IP: $TAP_IP"
echo ""

if command -v ssh >/dev/null 2>&1; then
    echo "--- SSH to VM ---"
    ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -p 22 root@"$TAP_IP" "hostname; uname -r; cat /etc/os-release | head -3" \
        2>/dev/null || echo "(SSH test failed — try manually)"
else
    echo "(ssh not found on host, testing via exec)"
    doas "$CHVM" exec chvm-ssh cat /etc/os-release
fi

echo ""
echo "To connect: ssh -p 2222 root@localhost  (password: chvm)"
echo "Or direct:  ssh root@$TAP_IP            (password: chvm)"
echo ""
echo "To stop: doas chvm rm -f chvm-ssh"
