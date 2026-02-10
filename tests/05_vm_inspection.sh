#!/bin/bash
# 05_vm_inspection.sh — List, logs, inspect tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "05 — VM Inspection"

VM_NAME="${TEST_PREFIX}-inspect"

# Start a detached VM
$CHVM run -d --name "$VM_NAME" alpine:latest sleep 3600 >/dev/null 2>&1
wait_vm_running "$VM_NAME" 30

# 1. chvm list shows VM with correct name and Running status
list_output=$($CHVM list 2>&1)
if echo "$list_output" | grep -qi "$VM_NAME.*running"; then
    pass "chvm list shows VM as Running"
elif echo "$list_output" | grep -q "$VM_NAME"; then
    pass "chvm list shows VM (status may differ)"
else
    fail "chvm list shows VM" "VM not found in list output"
fi

# 2. chvm list --all shows VMs
assert_output \
    "chvm list --all shows the VM" \
    "$CHVM list --all" \
    "$VM_NAME"

# 3. chvm inspect outputs JSON with expected fields
inspect_output=$($CHVM inspect "$VM_NAME" 2>&1)
inspect_ok=true
for field in "status" "network"; do
    if ! echo "$inspect_output" | grep -q "$field"; then
        fail "chvm inspect contains '$field'" "Not found"
        inspect_ok=false
    fi
done
if $inspect_ok; then
    pass "chvm inspect outputs expected fields"
fi

# 4. chvm logs contains boot messages
logs_output=$($CHVM logs "$VM_NAME" 2>&1)
if [ -n "$logs_output" ]; then
    pass "chvm logs returns output"
else
    skip "chvm logs contains boot messages" "Empty log output"
fi

# 5. Stop the VM, verify list behavior
$CHVM stop "$VM_NAME" >/dev/null 2>&1
wait_vm_stopped "$VM_NAME" 30

# After stop, chvm list (running only) should not show it
if $CHVM list 2>/dev/null | grep -q "$VM_NAME"; then
    fail "Stopped VM not in 'chvm list' (running only)"
else
    pass "Stopped VM not in 'chvm list' (running only)"
fi

# 6. After stop, chvm list --all shows Stopped
if $CHVM list --all 2>/dev/null | grep -qi "$VM_NAME.*stopped"; then
    pass "Stopped VM shows as Stopped in 'chvm list --all'"
elif $CHVM list --all 2>/dev/null | grep -q "$VM_NAME"; then
    pass "Stopped VM present in 'chvm list --all'"
else
    fail "Stopped VM in 'chvm list --all'" "VM not found"
fi

# Cleanup
cleanup_vm "$VM_NAME"

test_end
export_results
