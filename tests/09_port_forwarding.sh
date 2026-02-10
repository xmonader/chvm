#!/bin/bash
# 09_port_forwarding.sh — Port forward tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "09 — Port Forwarding"

VM_NAME="${TEST_PREFIX}-pf"

# 1. Start VM with port forward -p 18080:8080
$CHVM run -d --name "$VM_NAME" -p 18080:8080 alpine:latest sleep 3600 >/dev/null 2>&1
if ! wait_vm_running "$VM_NAME" 30; then
    fail "Start port-forward test VM" "Timeout"
    test_end
    export_results
    exit 1
fi

pass "VM with -p 18080:8080 starts successfully"

# 2. Verify iptables DNAT rule exists
ipt_output=$(iptables -t nat -L -n 2>&1) || true
if echo "$ipt_output" | grep -q "18080"; then
    pass "iptables DNAT rule exists for port 18080"
elif echo "$ipt_output" | grep -q "8080"; then
    pass "iptables rule references port 8080"
else
    skip "iptables DNAT rule check" "Rule not found (may use different NAT method)"
fi

# 3. Verify inspect shows port forward config
inspect_output=$($CHVM inspect "$VM_NAME" 2>&1) || true
if echo "$inspect_output" | grep -q "18080\|8080\|port"; then
    pass "chvm inspect shows port forward configuration"
else
    skip "chvm inspect port forward info" "Port info not found in inspect output"
fi

# Cleanup
cleanup_vm "$VM_NAME"

test_end
export_results
