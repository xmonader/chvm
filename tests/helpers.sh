#!/bin/bash
# helpers.sh — Shared test harness for CHVM automation tests

set -euo pipefail

# Binary path (prefer release, fall back to debug)
CHVM="${CHVM:-./target/x86_64-unknown-linux-musl/release/chvm}"
if [ ! -x "$CHVM" ]; then
    CHVM="./target/release/chvm"
fi
if [ ! -x "$CHVM" ]; then
    CHVM="./target/debug/chvm"
fi

# Counters
PASS=0
FAIL=0
SKIP=0

# Unique prefix for VM names to avoid collisions
TEST_PREFIX="test_$$"

# Default timeout for VM operations (seconds)
TIMEOUT="${TIMEOUT:-60}"

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m' # No Color

# --- Reporting ---

test_start() {
    local suite_name="$1"
    echo -e "\n${BOLD}${BLUE}=== $suite_name ===${NC}\n"
}

test_end() {
    echo ""
    echo -e "${BOLD}--- Suite Results ---${NC}"
    echo -e "  ${GREEN}PASS: $PASS${NC}  ${RED}FAIL: $FAIL${NC}  ${YELLOW}SKIP: $SKIP${NC}"
    echo ""
    return 0
}

pass() {
    local desc="$1"
    PASS=$((PASS + 1))
    echo -e "  ${GREEN}PASS${NC} $desc"
}

fail() {
    local desc="$1"
    shift
    FAIL=$((FAIL + 1))
    echo -e "  ${RED}FAIL${NC} $desc"
    if [ $# -gt 0 ]; then
        echo -e "       ${RED}$*${NC}"
    fi
}

skip() {
    local desc="$1"
    shift
    SKIP=$((SKIP + 1))
    echo -e "  ${YELLOW}SKIP${NC} $desc"
    if [ $# -gt 0 ]; then
        echo -e "       ${YELLOW}$*${NC}"
    fi
}

# --- Assertions ---

# Run command, expect exit 0
assert_success() {
    local desc="$1"
    local cmd="$2"
    local output
    if output=$(eval "$cmd" 2>&1); then
        pass "$desc"
    else
        fail "$desc" "Command failed (exit $?): $cmd"
        echo "       Output: $(echo "$output" | head -5)"
    fi
}

# Run command, expect non-zero exit
assert_failure() {
    local desc="$1"
    local cmd="$2"
    local output
    if output=$(eval "$cmd" 2>&1); then
        fail "$desc" "Expected failure but got exit 0: $cmd"
    else
        pass "$desc"
    fi
}

# Run command, check stdout contains substring
assert_output() {
    local desc="$1"
    local cmd="$2"
    local expected="$3"
    local output
    local rc=0
    output=$(eval "$cmd" 2>&1) || rc=$?
    if echo "$output" | grep -qF "$expected"; then
        pass "$desc"
    else
        fail "$desc" "Output missing '$expected'"
        echo "       Got: $(echo "$output" | head -5)"
    fi
}

# Run command, check specific exit code
assert_exit() {
    local desc="$1"
    local cmd="$2"
    local expected_code="$3"
    local rc=0
    eval "$cmd" >/dev/null 2>&1 || rc=$?
    if [ "$rc" -eq "$expected_code" ]; then
        pass "$desc"
    else
        fail "$desc" "Expected exit $expected_code, got $rc"
    fi
}

# --- VM Helpers ---

# Poll `chvm list` until VM status=running (or timeout)
wait_vm_running() {
    local name="$1"
    local timeout="${2:-$TIMEOUT}"
    local elapsed=0
    while [ $elapsed -lt $timeout ]; do
        if $CHVM list 2>/dev/null | grep -qi "$name.*running"; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done
    return 1
}

# Poll `chvm list --all` until VM status=stopped (or timeout)
wait_vm_stopped() {
    local name="$1"
    local timeout="${2:-$TIMEOUT}"
    local elapsed=0
    while [ $elapsed -lt $timeout ]; do
        if $CHVM list --all 2>/dev/null | grep -qi "$name.*stopped"; then
            return 0
        fi
        sleep 1
        elapsed=$((elapsed + 1))
    done
    return 1
}

# Force stop + rm a VM, ignore errors
cleanup_vm() {
    local name="$1"
    $CHVM stop "$name" >/dev/null 2>&1 || true
    $CHVM rm -f "$name" >/dev/null 2>&1 || true
}

# Remove all test VMs matching TEST_PREFIX
cleanup_all() {
    local vms
    vms=$($CHVM list --all 2>/dev/null | grep "$TEST_PREFIX" | awk '{print $2}' || true)
    for vm in $vms; do
        cleanup_vm "$vm"
    done
}

# Trap handler — cleanup on exit
trap_cleanup() {
    cleanup_all 2>/dev/null || true
}

# Export results to a temp file for aggregation by run_tests.sh
export_results() {
    local results_file="${RESULTS_FILE:-}"
    if [ -n "$results_file" ]; then
        echo "$PASS $FAIL $SKIP" > "$results_file"
    fi
    if [ "$FAIL" -gt 0 ]; then
        return 1
    fi
    return 0
}
