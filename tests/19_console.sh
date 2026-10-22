#!/bin/bash
# 19_console.sh — Serial rescue console tests (root needed, expect)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "19 — Serial Console"

VM_NAME="${TEST_PREFIX}-console"

if ! command -v expect >/dev/null 2>&1; then
    skip "serial console tests" "expect not installed"
    test_end
    export_results
    exit 0
fi

# Start detached VM
$CHVM run -d --name "$VM_NAME" alpine:latest sleep 3600 >/dev/null 2>&1
if ! wait_vm_running "$VM_NAME" 60; then
    fail "Start console test VM" "Timeout waiting for VM to start"
    test_end
    export_results
    exit 1
fi

# Wait for chvm-init to spawn the rescue shell on /dev/ttyS0
sleep 5

# Drive `chvm console` interactively: connect, run a command through the
# rescue shell, verify the output, detach with Ctrl-Q.
EXPECT_RESULTS=$(mktemp)
export EXPECT_RESULTS
trap 'rm -f "$EXPECT_RESULTS"' EXIT
EXPECT_RC=0
CHVM="$CHVM" VM_NAME="$VM_NAME" expect <<'EOF' || EXPECT_RC=$?
set timeout 60

set pass_count 0
set fail_count 0

set chvm $env(CHVM)
set vm_name $env(VM_NAME)

spawn $chvm console $vm_name

expect {
    "Press Ctrl-Q to exit" {
        puts "PASS: console connected"
        incr pass_count
    }
    timeout {
        puts "FAIL: console did not connect"
        incr fail_count
    }
}

# Give the rescue shell a moment to settle, then run a command
sleep 2
send "\recho CONSOLE_OK\r"
expect {
    "CONSOLE_OK" {
        puts "PASS: command output received over serial console"
        incr pass_count
    }
    timeout {
        puts "FAIL: no command output within timeout"
        incr fail_count
    }
}

# Ctrl-Q detaches from the console
send "\021"
expect {
    "Disconnected from serial console." {
        puts "PASS: Ctrl-Q detached from console"
        incr pass_count
    }
    timeout {
        puts "FAIL: Ctrl-Q did not detach"
        incr fail_count
    }
}
expect eof

puts ""
puts "--- Console Expect Results ---"
puts "  PASS: $pass_count  FAIL: $fail_count"

set rf [open $env(EXPECT_RESULTS) w]
puts $rf "$pass_count $fail_count"
close $rf

if {$fail_count > 0} {
    exit 1
}
exit 0
EOF

if [ "$EXPECT_RC" -ne 0 ]; then
    fail "serial console expect session" "expect script failed (exit $EXPECT_RC)"
elif [ -f "$EXPECT_RESULTS" ]; then
    read -r p f < "$EXPECT_RESULTS"
    PASS=$((PASS + p))
    FAIL=$((FAIL + f))
fi
rm -f "$EXPECT_RESULTS"

cleanup_vm "$VM_NAME"

test_end
export_results
exit 0
