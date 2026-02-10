#!/bin/bash
# 03_image_management.sh — OCI pull & cache tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

test_start "03 — Image Management"

# 1. Pull busybox:latest
assert_success \
    "chvm pull busybox:latest succeeds" \
    "$CHVM pull busybox:latest"

# 2. Pull alpine:latest
assert_success \
    "chvm pull alpine:latest succeeds" \
    "$CHVM pull alpine:latest"

# 3. chvm images lists pulled images
images_output=$($CHVM images 2>&1)
if echo "$images_output" | grep -q "busybox"; then
    pass "chvm images lists busybox"
else
    fail "chvm images lists busybox" "Not found in images output"
fi

if echo "$images_output" | grep -q "alpine"; then
    pass "chvm images lists alpine"
else
    fail "chvm images lists alpine" "Not found in images output"
fi

# 4. Re-pull same image — should be fast (cache hit)
start_time=$(date +%s)
$CHVM pull alpine:latest >/dev/null 2>&1
end_time=$(date +%s)
elapsed=$((end_time - start_time))
if [ "$elapsed" -le 10 ]; then
    pass "Re-pull alpine:latest uses cache (${elapsed}s)"
else
    fail "Re-pull alpine:latest uses cache" "Took ${elapsed}s — expected <=10s"
fi

# 5. Pull invalid image fails
assert_failure \
    "chvm pull invalid/does-not-exist:fake fails" \
    "$CHVM pull invalid/does-not-exist:fake"

test_end
export_results
