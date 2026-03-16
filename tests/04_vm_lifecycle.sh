#!/bin/bash
# 04_vm_lifecycle.sh — Core VM lifecycle tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "04 — VM Lifecycle"

# 1. Run foreground with --rm: output contains "hello"
assert_output \
    "Run foreground: chvm run --rm echo hello" \
    "$CHVM run --rm --name ${TEST_PREFIX}-fg alpine:latest echo hello" \
    "hello"

# 2. Run detached: returns VM ID, shows as Running
detach_output=$($CHVM run -d --name "${TEST_PREFIX}-bg" alpine:latest sleep 3600 2>&1)
if [ -n "$detach_output" ]; then
    pass "Run detached returns output (VM ID)"
else
    fail "Run detached returns output" "Empty output"
fi

if wait_vm_running "${TEST_PREFIX}-bg" 30; then
    pass "Detached VM shows as Running"
else
    fail "Detached VM shows as Running" "Timeout waiting for Running state"
fi

# 3. Stop the detached VM
$CHVM stop "${TEST_PREFIX}-bg" >/dev/null 2>&1
if wait_vm_stopped "${TEST_PREFIX}-bg" 30; then
    pass "chvm stop changes state to Stopped"
else
    fail "chvm stop changes state to Stopped" "Timeout"
fi

# 4. Remove the stopped VM
$CHVM rm "${TEST_PREFIX}-bg" >/dev/null 2>&1
if $CHVM list --all 2>/dev/null | grep -q "${TEST_PREFIX}-bg"; then
    fail "chvm rm removes VM from list" "VM still present"
else
    pass "chvm rm removes VM from list"
fi

# 5. Create + Start
$CHVM create --name "${TEST_PREFIX}-cs" alpine:latest sleep 3600 >/dev/null 2>&1
if $CHVM list --all 2>/dev/null | grep -q "${TEST_PREFIX}-cs"; then
    pass "chvm create adds VM to list"
else
    fail "chvm create adds VM to list"
fi

$CHVM start "${TEST_PREFIX}-cs" >/dev/null 2>&1
if wait_vm_running "${TEST_PREFIX}-cs" 30; then
    pass "chvm start transitions Created to Running"
else
    fail "chvm start transitions Created to Running" "Timeout"
fi

# 6. Force remove running VM
$CHVM rm -f "${TEST_PREFIX}-cs" >/dev/null 2>&1
if $CHVM list --all 2>/dev/null | grep -q "${TEST_PREFIX}-cs"; then
    fail "chvm rm -f removes running VM" "VM still present"
else
    pass "chvm rm -f removes running VM"
fi

# 7. Run --rm auto-cleans after exit
$CHVM run --rm --name "${TEST_PREFIX}-rm" alpine:latest echo done >/dev/null 2>&1
sleep 2
if $CHVM list --all 2>/dev/null | grep -q "${TEST_PREFIX}-rm"; then
    fail "run --rm auto-cleans VM" "VM still in list"
else
    pass "run --rm auto-cleans VM after exit"
fi

# 8. Stop → Start (restart cycle)
$CHVM run -d --name "${TEST_PREFIX}-restart" alpine:latest sleep 3600 >/dev/null 2>&1
wait_vm_running "${TEST_PREFIX}-restart" 30 || true
$CHVM stop "${TEST_PREFIX}-restart" >/dev/null 2>&1
wait_vm_stopped "${TEST_PREFIX}-restart" 30 || true

$CHVM start "${TEST_PREFIX}-restart" >/dev/null 2>&1
if wait_vm_running "${TEST_PREFIX}-restart" 30; then
    pass "chvm start after stop restarts VM"
else
    fail "chvm start after stop restarts VM" "Timeout waiting for Running state"
fi
cleanup_vm "${TEST_PREFIX}-restart"

# 9. Stop already-stopped VM → error
$CHVM run -d --name "${TEST_PREFIX}-stopped" alpine:latest sleep 3600 >/dev/null 2>&1
wait_vm_running "${TEST_PREFIX}-stopped" 30 || true
$CHVM stop "${TEST_PREFIX}-stopped" >/dev/null 2>&1
wait_vm_stopped "${TEST_PREFIX}-stopped" 30 || true
assert_failure \
    "Stop already-stopped VM errors" \
    "$CHVM stop ${TEST_PREFIX}-stopped"
cleanup_vm "${TEST_PREFIX}-stopped"

# 10. Remove non-existent VM → error
assert_failure \
    "Remove non-existent VM errors" \
    "$CHVM rm nonexistent_vm_${TEST_PREFIX}"

# 11. Prune removes stopped VMs
$CHVM run -d --name "${TEST_PREFIX}-prune1" alpine:latest sleep 3600 >/dev/null 2>&1
$CHVM run -d --name "${TEST_PREFIX}-prune2" alpine:latest sleep 3600 >/dev/null 2>&1
wait_vm_running "${TEST_PREFIX}-prune1" 30 || true
wait_vm_running "${TEST_PREFIX}-prune2" 30 || true
$CHVM stop "${TEST_PREFIX}-prune1" >/dev/null 2>&1
$CHVM stop "${TEST_PREFIX}-prune2" >/dev/null 2>&1
wait_vm_stopped "${TEST_PREFIX}-prune1" 30 || true
wait_vm_stopped "${TEST_PREFIX}-prune2" 30 || true

$CHVM prune >/dev/null 2>&1
prune_remaining=$($CHVM list --all 2>/dev/null | grep -c "${TEST_PREFIX}-prune" || true)
if [ "$prune_remaining" -eq 0 ]; then
    pass "chvm prune removes all stopped VMs"
else
    fail "chvm prune removes all stopped VMs" "$prune_remaining VMs remain"
    cleanup_vm "${TEST_PREFIX}-prune1"
    cleanup_vm "${TEST_PREFIX}-prune2"
fi

test_end
export_results
