#!/bin/bash
# 17_gc.sh — Garbage collection tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "17 — Garbage Collection"

# 1. doctor --gc with no orphans should succeed
output=$($CHVM doctor --gc 2>&1) || true
if echo "$output" | grep -q "No crashed VMs found"; then
    pass "doctor --gc reports no crashed VMs when none exist"
else
    fail "doctor --gc reports no crashed VMs" "Unexpected output"
    echo "       Output: $(echo "$output" | head -10)"
fi

# 2. Orphaned TAP device cleanup
# Create a fake orphaned TAP device
ip tuntap add dev tap-deadbeef mode tap 2>/dev/null || true
ip link set tap-deadbeef up 2>/dev/null || true

if ip link show tap-deadbeef >/dev/null 2>&1; then
    output=$($CHVM doctor --gc 2>&1) || true
    if ip link show tap-deadbeef >/dev/null 2>&1; then
        fail "doctor --gc removes orphaned TAP device" "tap-deadbeef still exists"
    else
        pass "doctor --gc removes orphaned TAP device"
    fi
else
    skip "Orphaned TAP cleanup" "Could not create test TAP device"
fi

# 3. Orphaned iptables rule cleanup (only when no VMs are running)
# Verify no VMs are running
running=$($CHVM list 2>/dev/null | grep -c "Running" || true)
if [ "$running" -eq 0 ]; then
    # Add a fake orphaned iptables rule
    iptables -t nat -A PREROUTING -p tcp --dport 59999 -j DNAT --to-destination 192.168.200.99:80 2>/dev/null || true
    if iptables -t nat -S 2>/dev/null | grep -q "192.168.200.99"; then
        output=$($CHVM doctor --gc 2>&1) || true
        if iptables -t nat -S 2>/dev/null | grep -q "192.168.200.99"; then
            fail "doctor --gc removes orphaned iptables rule" "Rule for 192.168.200.99 still exists"
        else
            pass "doctor --gc removes orphaned iptables rules"
        fi
    else
        skip "Orphaned iptables cleanup" "Could not create test iptables rule"
    fi
else
    skip "Orphaned iptables cleanup" "Running VMs exist, skipping to avoid interference"
fi

# 4. refresh_status cleans up resources when VM process dies
$CHVM run -d --name "${TEST_PREFIX}-gc-crash" alpine:latest sleep 3600 >/dev/null 2>&1 || true
if wait_vm_running "${TEST_PREFIX}-gc-crash" 30; then
    # Get the VMM PID
    vmm_pid=$($CHVM inspect "${TEST_PREFIX}-gc-crash" 2>/dev/null | grep -o '"vmm_pid":[0-9]*' | grep -o '[0-9]*' || true)
    vm_id=$($CHVM inspect "${TEST_PREFIX}-gc-crash" 2>/dev/null | grep -o '"id":"[^"]*"' | head -1 | grep -o '"[^"]*"$' | tr -d '"' || true)
    tap_prefix="${vm_id:0:8}"

    if [ -n "$vmm_pid" ] && [ "$vmm_pid" -gt 0 ]; then
        # Verify TAP device exists before kill
        tap_exists_before=false
        if ip link show "tap-${tap_prefix}" >/dev/null 2>&1; then
            tap_exists_before=true
        fi

        # Kill the cloud-hypervisor process (simulate crash)
        kill -9 "$vmm_pid" 2>/dev/null || true
        sleep 1

        # Run chvm list which triggers refresh_status with cleanup
        $CHVM list --all >/dev/null 2>&1 || true

        # Verify TAP device was cleaned up
        if $tap_exists_before; then
            if ip link show "tap-${tap_prefix}" >/dev/null 2>&1; then
                fail "refresh_status cleans up TAP on crash" "tap-${tap_prefix} still exists"
            else
                pass "refresh_status cleans up TAP device when VM process dies"
            fi
        else
            skip "TAP cleanup on crash" "TAP device was not created"
        fi

        # Verify VM is marked as stopped
        if $CHVM list --all 2>/dev/null | grep -qi "${TEST_PREFIX}-gc-crash.*stopped"; then
            pass "refresh_status marks crashed VM as stopped"
        else
            fail "refresh_status marks crashed VM as stopped" "VM not in stopped state"
        fi
    else
        skip "Crash cleanup test" "Could not get VMM PID"
    fi
else
    skip "Crash cleanup test" "VM did not start"
fi
cleanup_vm "${TEST_PREFIX}-gc-crash"

test_end
export_results
