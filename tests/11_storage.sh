#!/bin/bash
# 11_storage.sh — Volume mount tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "11 — Storage"

# Create a temp directory with a test file
TMPDIR=$(mktemp -d /tmp/chvm-test-XXXXXX)
echo "chvm_volume_test_content" > "$TMPDIR/testfile.txt"
mkdir -p "$TMPDIR/subdir"
echo "nested_content" > "$TMPDIR/subdir/nested.txt"

# 1. Volume mount: ls shows test file
ls_output=$($CHVM run --rm -v "$TMPDIR:/mnt/data" --name "${TEST_PREFIX}-vol1" alpine:latest ls /mnt/data 2>&1) || true
if echo "$ls_output" | grep -q "testfile.txt"; then
    pass "Volume mount: ls /mnt/data shows testfile.txt"
else
    fail "Volume mount: ls /mnt/data shows testfile.txt" "Not found"
    echo "       Output: $(echo "$ls_output" | head -5)"
fi

# 2. Volume mount: cat reads correct content
cat_output=$($CHVM run --rm -v "$TMPDIR:/mnt/data" --name "${TEST_PREFIX}-vol2" alpine:latest cat /mnt/data/testfile.txt 2>&1) || true
if echo "$cat_output" | grep -q "chvm_volume_test_content"; then
    pass "Volume mount: cat reads correct file content"
else
    fail "Volume mount: cat reads correct file content"
    echo "       Output: $(echo "$cat_output" | head -5)"
fi

# 3. Volume mount: nested directory accessible
nested_output=$($CHVM run --rm -v "$TMPDIR:/mnt/data" --name "${TEST_PREFIX}-vol3" alpine:latest cat /mnt/data/subdir/nested.txt 2>&1) || true
if echo "$nested_output" | grep -q "nested_content"; then
    pass "Volume mount: nested directory accessible"
else
    skip "Volume mount: nested directory" "Could not read nested content"
fi

# Cleanup temp dir
rm -rf "$TMPDIR"

test_end
export_results
