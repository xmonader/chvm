#!/bin/bash
# 18_stats.sh — Stats command tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "18 — VM Stats"

VM_NAME="${TEST_PREFIX}-stats"

# Start a detached VM
$CHVM run -d --name "$VM_NAME" alpine:latest sleep 3600 >/dev/null 2>&1
wait_vm_running "$VM_NAME" 30

# 1. chvm stats succeeds on running VM
assert_success \
    "chvm stats succeeds on running VM" \
    "$CHVM stats $VM_NAME"

# 2. chvm stats shows CPU section
assert_output \
    "chvm stats shows CPU info" \
    "$CHVM stats $VM_NAME" \
    "CPU"

# 3. chvm stats shows MEMORY section
assert_output \
    "chvm stats shows MEMORY info" \
    "$CHVM stats $VM_NAME" \
    "MEMORY"

# 4. chvm stats shows NETWORK section
assert_output \
    "chvm stats shows NETWORK info" \
    "$CHVM stats $VM_NAME" \
    "NETWORK"

# 5. chvm stats --json outputs valid JSON
json_output=$($CHVM stats "$VM_NAME" --json 2>&1)
if echo "$json_output" | jq . >/dev/null 2>&1; then
    pass "chvm stats --json outputs valid JSON"
else
    fail "chvm stats --json outputs valid JSON" "Invalid JSON output"
fi

# 6. JSON contains expected fields
for field in "id" "name" "vcpu" "memory_mb" "network"; do
    if echo "$json_output" | jq -e ".$field" >/dev/null 2>&1; then
        pass "JSON contains '$field' field"
    else
        fail "JSON contains '$field' field" "Not found"
    fi
done

# 7. Stats on stopped VM fails
$CHVM stop "$VM_NAME" >/dev/null 2>&1
wait_vm_stopped "$VM_NAME" 30

assert_failure \
    "chvm stats fails on stopped VM" \
    "$CHVM stats $VM_NAME"

# Cleanup
cleanup_vm "$VM_NAME"

test_end
export_results
