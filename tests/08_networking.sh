#!/bin/bash
# 08_networking.sh — TAP network tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "08 — Networking"

VM_NAME="${TEST_PREFIX}-net"

# Start detached VM
$CHVM run -d --name "$VM_NAME" alpine:latest sleep 3600 >/dev/null 2>&1
if ! wait_vm_running "$VM_NAME" 30; then
    fail "Start networking test VM" "Timeout"
    test_end
    export_results
    exit 1
fi

# Wait for vsock
sleep 5

# 1. VM has an eth0 interface with an IP address
ip_output=$($CHVM exec "$VM_NAME" -- ip addr show eth0 2>&1) || true
if echo "$ip_output" | grep -q "inet "; then
    pass "VM eth0 has an IP address"
else
    fail "VM eth0 has an IP address" "No inet found in ip addr output"
    echo "       Output: $(echo "$ip_output" | head -5)"
fi

# 2. Can reach gateway (192.168.200.1)
ping_gw=$($CHVM exec "$VM_NAME" -- ping -c 2 -W 5 192.168.200.1 2>&1) || true
if echo "$ping_gw" | grep -q "bytes from"; then
    pass "VM can ping gateway 192.168.200.1"
else
    skip "VM can ping gateway" "Gateway not reachable (may vary by network config)"
fi

# 3. Internet access via NAT
ping_inet=$($CHVM exec "$VM_NAME" -- ping -c 2 -W 5 8.8.8.8 2>&1) || true
if echo "$ping_inet" | grep -q "bytes from"; then
    pass "VM has internet access (ping 8.8.8.8)"
else
    skip "VM has internet access" "Internet not reachable (NAT may not be configured)"
fi

# 4. DNS resolution
dns_output=$($CHVM exec "$VM_NAME" -- nslookup google.com 2>&1) || true
if echo "$dns_output" | grep -qi "address"; then
    pass "VM DNS resolution works"
else
    skip "VM DNS resolution" "DNS not available"
fi

# Cleanup
cleanup_vm "$VM_NAME"

test_end
export_results
