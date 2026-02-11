#!/bin/bash
# Build and run the alpine-ssh image with chvm over mycelium
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
IMAGE_NAME="localhost/chvm-alpine-ssh:latest"
CHVM="$(command -v chvm)"

PEERS="tcp://188.40.132.242:9651,tcp://136.243.47.186:9651,tcp://185.69.166.7:9651,tcp://65.21.231.58:9651,tcp://209.159.146.190:9651"

# Build only if Docker doesn't have the image
if ! docker image inspect "$IMAGE_NAME" >/dev/null 2>&1; then
    echo "=== Building alpine-ssh image ==="
    docker build -t "$IMAGE_NAME" "$SCRIPT_DIR"
else
    echo "=== Image $IMAGE_NAME already exists in Docker ==="
fi

echo ""
echo "=== Running with chvm (mycelium network) ==="
doas "$CHVM" run -d --name chvm-ssh-myc --network mycelium --mycelium-peers "$PEERS" "$IMAGE_NAME"

echo ""
echo "Waiting for sshd + mycelium to start..."
sleep 10

MYC_IP=$(doas "$CHVM" inspect chvm-ssh-myc 2>/dev/null | grep '"mycelium_ip"' | head -1 | sed 's/.*: *"\([^"]*\)".*/\1/')

echo ""
echo "=== Info ==="
echo "Mycelium IP: $MYC_IP"
echo ""

if [ -n "$MYC_IP" ]; then
    echo "--- Testing SSH over mycelium ---"
    ssh -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
        -o ConnectTimeout=10 \
        root@"$MYC_IP" "echo 'Hello from chvm over mycelium!'; hostname; uname -r" \
        2>/dev/null || echo "(SSH over mycelium failed — route may still be converging, try again in a few seconds)"
fi

echo ""
echo "To connect: ssh root@$MYC_IP  (password: chvm)"
echo ""
echo "This address is reachable from ANY device on the mycelium network."
echo ""
echo "To stop: doas chvm rm -f chvm-ssh-myc"
