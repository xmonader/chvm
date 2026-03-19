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

# 2. doctor --gc should only remove stale resources CHVM can tie back to a stopped VM
GC_VM="${TEST_PREFIX}-gc-owned"
$CHVM run -d --name "$GC_VM" -p 59999:80 alpine:latest sleep 3600 >/dev/null 2>&1 || true
if wait_vm_running "$GC_VM" 30; then
    tap_device=$($CHVM inspect "$GC_VM" 2>/dev/null | grep -o '"tap_device": *"[^"]*"' | head -1 | cut -d'"' -f4 || true)
    ip_address=$($CHVM inspect "$GC_VM" 2>/dev/null | grep -o '"ip_address": *"[^"]*"' | head -1 | cut -d'"' -f4 | cut -d/ -f1 || true)

    $CHVM stop "$GC_VM" >/dev/null 2>&1 || true
    wait_vm_stopped "$GC_VM" 30 || true

    # Recreate resources that belong to the stopped VM; conservative GC should remove only these.
    if [ -n "$tap_device" ]; then
        ip tuntap add dev "$tap_device" mode tap 2>/dev/null || true
        ip link set "$tap_device" up 2>/dev/null || true
    fi
    if [ -n "$ip_address" ]; then
        iptables -t nat -A PREROUTING -p tcp --dport 59999 -j DNAT --to-destination "${ip_address}:80" 2>/dev/null || true
    fi

    output=$($CHVM doctor --gc 2>&1) || true

    if [ -n "$tap_device" ] && ip link show "$tap_device" >/dev/null 2>&1; then
        fail "doctor --gc removes orphaned TAP device tied to stopped VM" "${tap_device} still exists"
    else
        pass "doctor --gc removes orphaned TAP device tied to stopped VM"
    fi

    if [ -n "$ip_address" ] && iptables -t nat -S 2>/dev/null | grep -q "$ip_address"; then
        fail "doctor --gc removes orphaned iptables rule tied to stopped VM" "Rule for ${ip_address} still exists"
    else
        pass "doctor --gc removes orphaned iptables rule tied to stopped VM"
    fi
else
    skip "doctor --gc removes CHVM-owned orphaned resources" "VM did not start"
fi
cleanup_vm "$GC_VM"

# 4. refresh_status cleans up resources when VM process dies
$CHVM run -d --name "${TEST_PREFIX}-gc-crash" alpine:latest sleep 3600 >/dev/null 2>&1 || true
if wait_vm_running "${TEST_PREFIX}-gc-crash" 30; then
        # Get the VMM PID
        vmm_pid=$($CHVM inspect "${TEST_PREFIX}-gc-crash" 2>/dev/null | grep -Eo '"vmm_pid": *[0-9]+' | grep -Eo '[0-9]+' || true)
        vm_id=$($CHVM inspect "${TEST_PREFIX}-gc-crash" 2>/dev/null | grep -Eo '"id": *"[^"]*"' | head -1 | cut -d'"' -f4 || true)
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
