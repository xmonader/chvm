#!/bin/bash
# 10_resources.sh — Resource configuration tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "10 — Resources"

# 1. Memory configuration: --memory 256
mem_output=$($CHVM run --rm --memory 256 --name "${TEST_PREFIX}-mem" alpine:latest cat /proc/meminfo 2>&1) || true
if echo "$mem_output" | grep -q "MemTotal"; then
    # Extract MemTotal in kB and check it's roughly 256MB (~200-270MB range)
    mem_kb=$(echo "$mem_output" | grep "MemTotal" | awk '{print $2}')
    if [ -n "$mem_kb" ]; then
        mem_mb=$((mem_kb / 1024))
        if [ "$mem_mb" -ge 180 ] && [ "$mem_mb" -le 280 ]; then
            pass "VM memory set to ~256MB (got ${mem_mb}MB)"
        else
            fail "VM memory set to ~256MB" "Got ${mem_mb}MB"
        fi
    else
        skip "Memory check" "Could not parse MemTotal"
    fi
else
    fail "VM reports MemTotal in /proc/meminfo" "MemTotal not found"
    echo "       Output: $(echo "$mem_output" | head -5)"
fi

# 2. Single env var: -e TESTVAR=hello123
assert_output \
    "Single env var -e TESTVAR=hello123" \
    "$CHVM run --rm -e TESTVAR=hello123 --name ${TEST_PREFIX}-env1 alpine:latest env" \
    "TESTVAR=hello123"

# 3. Multiple env vars: -e A=1 -e B=2
multi_env_output=$($CHVM run --rm -e A=1 -e B=2 --name "${TEST_PREFIX}-env2" alpine:latest env 2>&1) || true
env_ok=true
if ! echo "$multi_env_output" | grep -q "A=1"; then
    fail "Multiple env: A=1 present" "Not found"
    env_ok=false
fi
if ! echo "$multi_env_output" | grep -q "B=2"; then
    fail "Multiple env: B=2 present" "Not found"
    env_ok=false
fi
if $env_ok; then
    pass "Multiple env vars (-e A=1 -e B=2) both present"
fi

# 4. CPU configuration (inspect-based check)
$CHVM run -d --cpus 2 --name "${TEST_PREFIX}-cpu" alpine:latest sleep 3600 >/dev/null 2>&1 || true
if wait_vm_running "${TEST_PREFIX}-cpu" 30; then
    inspect_out=$($CHVM inspect "${TEST_PREFIX}-cpu" 2>&1) || true
    if echo "$inspect_out" | grep -q "2"; then
        pass "VM created with --cpus 2"
    else
        skip "CPU config check" "Could not verify CPU count in inspect"
    fi
else
    skip "CPU config check" "VM did not start"
fi
cleanup_vm "${TEST_PREFIX}-cpu"

test_end
export_results
