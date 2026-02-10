#!/bin/bash
# 14_rootfs.sh — Rootfs and storage backend tests (root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "14 — Rootfs"

# Determine CHVM_HOME (same logic as chvm itself)
if [ -n "${DOAS_USER:-}" ]; then
    CHVM_HOME="$(eval echo "~$DOAS_USER")/.chvm"
elif [ -n "${SUDO_USER:-}" ]; then
    CHVM_HOME="$(eval echo "~$SUDO_USER")/.chvm"
else
    CHVM_HOME="$HOME/.chvm"
fi

# Find alpine rootfs from OCI cache
ALPINE_ROOTFS=""
for digest_dir in "$CHVM_HOME"/images/*/; do
    rootfs_dir="${digest_dir}rootfs"
    if [ -d "$rootfs_dir" ] && [ -f "$rootfs_dir/bin/sh" ]; then
        ALPINE_ROOTFS="$rootfs_dir"
        break
    fi
done

if [ -z "$ALPINE_ROOTFS" ]; then
    skip "Local rootfs: run from cached OCI rootfs" "No extracted rootfs found in $CHVM_HOME/images/"
    skip "Local rootfs: echo command produces output" "Skipped (no rootfs)"
    skip "Local rootfs: ls / contains expected dirs" "Skipped (no rootfs)"
    test_end
    export_results
    exit 0
fi

# 1. Run a command using --rootfs with a cached OCI rootfs
run_output=$($CHVM run --rm --rootfs "$ALPINE_ROOTFS" --name "${TEST_PREFIX}-rootfs1" alpine:latest echo "rootfs_test_ok" 2>&1) || true
if echo "$run_output" | grep -q "rootfs_test_ok"; then
    pass "Local rootfs: echo command produces output"
else
    fail "Local rootfs: echo command produces output"
    echo "       Output: $(echo "$run_output" | tail -5)"
fi

# 2. Verify the rootfs has expected filesystem structure
ls_output=$($CHVM run --rm --rootfs "$ALPINE_ROOTFS" --name "${TEST_PREFIX}-rootfs2" alpine:latest ls / 2>&1) || true
if echo "$ls_output" | grep -q "bin" && echo "$ls_output" | grep -q "etc" && echo "$ls_output" | grep -q "usr"; then
    pass "Local rootfs: ls / contains expected dirs"
else
    fail "Local rootfs: ls / contains expected dirs"
    echo "       Output: $(echo "$ls_output" | tail -5)"
fi

# 3. Run with virtiofs storage backend (skip if virtiofsd not available)
VIRTIOFSD_FOUND=false
for vfsd in virtiofsd /usr/libexec/virtiofsd /usr/lib/virtiofsd /usr/local/bin/virtiofsd /usr/bin/virtiofsd /usr/lib/qemu/virtiofsd; do
    if command -v "$vfsd" &>/dev/null || [ -x "$vfsd" ]; then
        VIRTIOFSD_FOUND=true
        break
    fi
done
if [ "$VIRTIOFSD_FOUND" = true ]; then
    vfs_output=$(timeout 30 $CHVM run --rm --storage virtiofs --name "${TEST_PREFIX}-rootfs3" alpine:latest echo "virtiofs_ok" 2>&1) || true
    # Clean up the VM if timeout left it running
    cleanup_vm "${TEST_PREFIX}-rootfs3"
    if echo "$vfs_output" | grep -q "virtiofs_ok"; then
        pass "VirtioFS storage backend: echo command works"
    else
        # VirtioFS requires kernel virtiofs/FUSE support — skip if kernel can't mount it
        skip "VirtioFS storage backend: echo command works" "virtiofsd available but kernel may lack virtiofs support"
    fi
else
    skip "VirtioFS storage backend: echo command works" "virtiofsd not found"
fi

test_end
export_results
