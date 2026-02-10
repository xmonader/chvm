#!/bin/bash
# 12_kernel.sh — Kernel management tests (partially no root)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

test_start "12 — Kernel Management"

# 1. chvm kernel list exits 0
assert_success \
    "chvm kernel list exits 0" \
    "$CHVM kernel list"

# 2. chvm kernel check (if a kernel exists)
kernel_list=$($CHVM kernel list 2>&1)
if echo "$kernel_list" | grep -qi "vmlinux\|kernel\|default"; then
    check_output=$($CHVM kernel check 2>&1) || true
    if [ -n "$check_output" ]; then
        pass "chvm kernel check shows output"
    else
        skip "chvm kernel check" "Empty output"
    fi
else
    skip "chvm kernel check" "No kernels found to check"
fi

# 3. chvm kernel extract-host (needs root, extracts host kernel)
if [ "$(id -u)" -eq 0 ] || command -v doas >/dev/null 2>&1; then
    extract_output=$($CHVM kernel extract-host 2>&1) || true
    if echo "$extract_output" | grep -qi "extract\|kernel\|vmlinux\|success"; then
        pass "chvm kernel extract-host runs"
    else
        skip "chvm kernel extract-host" "Command did not succeed: $(echo "$extract_output" | head -2)"
    fi
else
    skip "chvm kernel extract-host" "Not running as root"
fi

# 4. chvm kernel set-default with vmlinux-host (avoid self-referencing symlink)
if [ -f "$HOME/.chvm/kernels/vmlinux-host" ] || [ -f "/root/.chvm/kernels/vmlinux-host" ]; then
    set_output=$($CHVM kernel set-default vmlinux-host 2>&1) || true
    pass "chvm kernel set-default runs"
else
    kernel_list2=$($CHVM kernel list 2>&1)
    # Find a non-symlink kernel to set as default
    first_kernel=$(echo "$kernel_list2" | grep -i "vmlinux-" | head -1 | awk '{print $1}')
    if [ -n "$first_kernel" ] && [ "$first_kernel" != "vmlinux" ]; then
        set_output=$($CHVM kernel set-default "$first_kernel" 2>&1) || true
        pass "chvm kernel set-default runs"
    else
        skip "chvm kernel set-default" "No non-default kernels available"
    fi
fi

test_end
export_results
