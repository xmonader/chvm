#!/bin/bash
# 06_exec.sh — Command execution via vsock tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "06 — Exec"

VM_NAME="${TEST_PREFIX}-exec"

# Start detached VM
$CHVM run -d --name "$VM_NAME" alpine:latest sleep 3600 >/dev/null 2>&1
if ! wait_vm_running "$VM_NAME" 30; then
    fail "Start exec test VM" "Timeout waiting for VM to start"
    test_end
    export_results
    exit 1
fi

# Wait a bit for vsock to be ready
sleep 5

# 1. exec echo hello
assert_output \
    "exec echo hello returns 'hello'" \
    "$CHVM exec $VM_NAME -- echo hello" \
    "hello"

# 2. exec ls / — contains standard dirs
ls_output=$($CHVM exec "$VM_NAME" -- ls / 2>&1)
ls_ok=true
for dir in bin etc usr; do
    if ! echo "$ls_output" | grep -q "$dir"; then
        fail "exec ls / contains '$dir'" "Not found"
        ls_ok=false
    fi
done
if $ls_ok; then
    pass "exec ls / contains bin, etc, usr"
fi

# 3. exec with -e env var
assert_output \
    "exec -e FOO=bar sets environment variable" \
    "$CHVM exec -e FOO=bar $VM_NAME -- env" \
    "FOO=bar"

# 4. exec with -w workdir
assert_output \
    "exec -w /tmp sets working directory" \
    "$CHVM exec -w /tmp $VM_NAME -- pwd" \
    "/tmp"

# 5. exec false — exits non-zero
assert_failure \
    "exec false exits non-zero" \
    "$CHVM exec $VM_NAME -- false"

# Cleanup
cleanup_vm "$VM_NAME"

test_end
export_results
