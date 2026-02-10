#!/bin/bash
# 13_mycelium.sh — Mycelium networking tests (root needed)
# Tests: mycelium interface creation, IPv6 address assignment, VM-to-VM ping
#
# NOTE: VM-to-VM ping via mycelium requires route convergence through a shared
# peer. With a public peer, this takes ~60 seconds. The ping tests use retries
# with backoff to account for this.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "13 — Mycelium Networking"

# --- Prerequisites ---

# Check mycelium binary exists (chvm handles finding it during image creation)
REAL_USER="${SUDO_USER:-${DOAS_USER:-}}"
REAL_HOME="$HOME"
if [ -n "$REAL_USER" ]; then
    REAL_HOME=$(eval echo "~$REAL_USER" 2>/dev/null || echo "$HOME")
fi

MYCELIUM_FOUND=false
for candidate in "$REAL_HOME/.chvm/bin/mycelium" "$HOME/.chvm/bin/mycelium" /usr/local/bin/mycelium /usr/bin/mycelium ./bin/mycelium; do
    if [ -x "$candidate" ]; then
        MYCELIUM_FOUND=true
        pass "Mycelium binary available ($candidate)"
        break
    fi
done

if ! $MYCELIUM_FOUND; then
    skip "Mycelium binary available" "mycelium not found — skipping all mycelium tests"
    test_end || true
    export_results
    exit 0
fi

# --- Start two VMs with mycelium pointing to public peers ---

VM1_NAME="${TEST_PREFIX}-myc1"
VM2_NAME="${TEST_PREFIX}-myc2"
# Use multiple public peers for faster route convergence
# Full list: https://github.com/threefoldtech/mycelium#hosted-public-nodes
PUBLIC_PEERS="tcp://188.40.132.242:9651,tcp://136.243.47.186:9651,tcp://185.69.166.7:9651,tcp://65.21.231.58:9651,tcp://209.159.146.190:9651"

cleanup_mycelium() {
    cleanup_vm "$VM1_NAME" 2>/dev/null || true
    cleanup_vm "$VM2_NAME" 2>/dev/null || true
}
trap cleanup_mycelium EXIT

# Split comma-separated peers into separate --mycelium-peers flags
PEER_FLAGS=""
IFS=',' read -ra PEER_ARRAY <<< "$PUBLIC_PEERS"
for p in "${PEER_ARRAY[@]}"; do
    PEER_FLAGS="$PEER_FLAGS --mycelium-peers $p"
done

$CHVM run -d --name "$VM1_NAME" \
    --network mycelium \
    $PEER_FLAGS \
    alpine:latest sleep 3600 >/dev/null 2>&1

if wait_vm_running "$VM1_NAME" 30; then
    pass "VM1 started with mycelium networking"
else
    fail "VM1 started with mycelium networking" "Timeout"
    test_end || true
    export_results
    exit 0
fi

$CHVM run -d --name "$VM2_NAME" \
    --network mycelium \
    $PEER_FLAGS \
    alpine:latest sleep 3600 >/dev/null 2>&1

if wait_vm_running "$VM2_NAME" 30; then
    pass "VM2 started with mycelium networking"
else
    fail "VM2 started with mycelium networking" "Timeout"
    test_end || true
    export_results
    exit 0
fi

# Wait for vsock + mycelium startup
sleep 8

# --- Test: VM1 has mycelium interface ---

vm1_ifaces=$($CHVM exec "$VM1_NAME" -- ip link show 2>&1) || true
if echo "$vm1_ifaces" | grep -q "mycelium"; then
    pass "VM1 has mycelium TUN interface"
else
    fail "VM1 has mycelium TUN interface" "No mycelium interface found"
    echo "       Output: $(echo "$vm1_ifaces" | head -10)"
fi

# --- Test: VM2 has mycelium interface ---

vm2_ifaces=$($CHVM exec "$VM2_NAME" -- ip link show 2>&1) || true
if echo "$vm2_ifaces" | grep -q "mycelium"; then
    pass "VM2 has mycelium TUN interface"
else
    fail "VM2 has mycelium TUN interface" "No mycelium interface found"
    echo "       Output: $(echo "$vm2_ifaces" | head -10)"
fi

# --- Test: Get VM1 mycelium IPv6 address ---

VM1_IP=""
vm1_addr=$($CHVM exec "$VM1_NAME" -- ip -6 addr show dev mycelium scope global 2>&1) || true
if echo "$vm1_addr" | grep -q "inet6"; then
    VM1_IP=$(echo "$vm1_addr" | grep "inet6" | head -1 | awk '{print $2}' | cut -d/ -f1)
    pass "VM1 has mycelium IPv6 address ($VM1_IP)"
else
    fail "VM1 has mycelium IPv6 address" "No global inet6 on mycelium"
    echo "       Output: $(echo "$vm1_addr" | head -5)"
fi

# --- Test: Get VM2 mycelium IPv6 address ---

VM2_IP=""
vm2_addr=$($CHVM exec "$VM2_NAME" -- ip -6 addr show dev mycelium scope global 2>&1) || true
if echo "$vm2_addr" | grep -q "inet6"; then
    VM2_IP=$(echo "$vm2_addr" | grep "inet6" | head -1 | awk '{print $2}' | cut -d/ -f1)
    pass "VM2 has mycelium IPv6 address ($VM2_IP)"
else
    fail "VM2 has mycelium IPv6 address" "No global inet6 on mycelium"
    echo "       Output: $(echo "$vm2_addr" | head -5)"
fi

# --- Test: VM1 and VM2 have different addresses ---

if [ -n "$VM1_IP" ] && [ -n "$VM2_IP" ]; then
    if [ "$VM1_IP" != "$VM2_IP" ]; then
        pass "VM1 and VM2 have different mycelium addresses"
    else
        fail "VM1 and VM2 have different mycelium addresses" "Both have $VM1_IP"
    fi
fi

# --- Test: Both VMs connected to public peer ---

vm1_peer=$($CHVM exec "$VM1_NAME" -- netstat -tn 2>&1) || true
if echo "$vm1_peer" | grep -q ":9651.*ESTABLISHED"; then
    pass "VM1 connected to public mycelium peer"
else
    skip "VM1 connected to public mycelium peer" "No ESTABLISHED connection to any peer"
fi

vm2_peer=$($CHVM exec "$VM2_NAME" -- netstat -tn 2>&1) || true
if echo "$vm2_peer" | grep -q ":9651.*ESTABLISHED"; then
    pass "VM2 connected to public mycelium peer"
else
    skip "VM2 connected to public mycelium peer" "No ESTABLISHED connection to any peer"
fi

# --- Test: VM1 can ping VM2 via mycelium ---
# Mycelium route convergence through public peers takes ~60s.
# We retry with increasing delays to allow for route discovery.

ping_ok=false
if [ -n "$VM2_IP" ]; then
    for attempt in 1 2 3 4 5 6; do
        ping_output=$($CHVM exec "$VM1_NAME" -- ping -6 -c 3 -W 15 "$VM2_IP" 2>&1) || true
        if echo "$ping_output" | grep -q "bytes from"; then
            ping_ok=true
            break
        fi
        # Wait for routes to propagate through the public peer
        sleep 15
    done
    if $ping_ok; then
        pass "VM1 can ping VM2 via mycelium ($VM2_IP)"
    else
        skip "VM1 can ping VM2 via mycelium" "Route convergence too slow (tried ${attempt}x, ~90s)"
    fi
else
    skip "VM1 can ping VM2 via mycelium" "VM2 address not available"
fi

# --- Test: VM2 can ping VM1 via mycelium ---

if [ -n "$VM1_IP" ] && $ping_ok; then
    ping_output=$($CHVM exec "$VM2_NAME" -- ping -6 -c 3 -W 15 "$VM1_IP" 2>&1) || true
    if echo "$ping_output" | grep -q "bytes from"; then
        pass "VM2 can ping VM1 via mycelium ($VM1_IP)"
    else
        skip "VM2 can ping VM1 via mycelium" "Reverse route not yet converged"
    fi
elif [ -n "$VM1_IP" ]; then
    skip "VM2 can ping VM1 via mycelium" "Skipped (forward ping didn't succeed)"
else
    skip "VM2 can ping VM1 via mycelium" "VM1 address not available"
fi

# --- Test: Mycelium address visible via inspect ---

vm1_inspect_ip=$($CHVM inspect "$VM1_NAME" 2>&1) || true
if echo "$vm1_inspect_ip" | grep -q "mycelium_ip"; then
    pass "Mycelium IP visible in chvm inspect"
else
    fail "Mycelium IP visible in chvm inspect" "mycelium_ip not found in inspect output"
fi

# Cleanup
cleanup_vm "$VM1_NAME"
cleanup_vm "$VM2_NAME"

test_end
export_results
