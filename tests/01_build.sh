#!/bin/bash
# 01_build.sh — Build verification tests (no root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

test_start "01 — Build Verification"

# 1. cargo build succeeds (skip if cargo not available, e.g. running as root)
if cargo --version >/dev/null 2>&1; then
    assert_success \
        "cargo build --workspace succeeds" \
        "cargo build --workspace --release 2>&1"
else
    skip "cargo build --workspace succeeds" "cargo not available (running as root?)"
fi

# 2. Binary exists
if [ -x "./target/release/chvm" ]; then
    pass "Binary target/release/chvm exists and is executable"
elif [ -x "./target/debug/chvm" ]; then
    pass "Binary target/debug/chvm exists and is executable"
else
    fail "No chvm binary found in target/release or target/debug"
fi

# 3. chvm --help exits 0 and contains expected content
assert_output \
    "chvm --help exits 0 and mentions Cloud Hypervisor" \
    "$CHVM --help" \
    "Cloud Hypervisor"

# 4. chvm version shows version string
assert_output \
    "chvm version shows version string" \
    "$CHVM version" \
    "chvm"

test_end
export_results
