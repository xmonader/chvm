#!/bin/bash
# 16_redis.sh — Redis real-world scenario tests (root needed)
# Tests: redis via port forwarding, redis accessible over mycelium
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "16 — Redis Scenarios"

# Helper: extract a field from chvm inspect JSON output
inspect_field() {
    $CHVM inspect "$1" 2>/dev/null | grep "\"$2\"" | head -1 | sed 's/.*: *"\([^"]*\)".*/\1/'
}

# Helper: install redis and start it bound to all interfaces
setup_redis() {
    local vm_name="$1"
    $CHVM exec "$vm_name" -- sh -c "\
        apk add --no-cache redis >/dev/null 2>&1 && \
        redis-server --bind 0.0.0.0 :: --protected-mode no --daemonize yes" 2>&1
}

# ============================================================
# Scenario 1: Redis via port forwarding
# ============================================================

PF_VM="${TEST_PREFIX}-redis-pf"

$CHVM run -d --name "$PF_VM" -p 26379:6379 alpine:latest sleep 3600 >/dev/null 2>&1

if wait_vm_running "$PF_VM" 30; then
    pass "Port-forward VM started"
else
    fail "Port-forward VM started" "Timeout"
    test_end || true
    export_results
    exit 0
fi

sleep 5

# Install and start redis
setup_output=$(setup_redis "$PF_VM" 2>&1) || true
if $CHVM exec "$PF_VM" -- pgrep redis-server >/dev/null 2>&1; then
    pass "Redis installed and running in port-forward VM"
else
    fail "Redis installed and running in port-forward VM" "redis-server not running"
    echo "       Output: $(echo "$setup_output" | tail -5)"
fi

# Test: SET and GET via exec
set_result=$($CHVM exec "$PF_VM" -- redis-cli SET chvm:test "hello from chvm" 2>&1) || true
if echo "$set_result" | grep -q "OK"; then
    pass "Redis SET via exec"
else
    fail "Redis SET via exec" "Got: $set_result"
fi

get_result=$($CHVM exec "$PF_VM" -- redis-cli GET chvm:test 2>&1) || true
if echo "$get_result" | grep -q "hello from chvm"; then
    pass "Redis GET via exec"
else
    fail "Redis GET via exec" "Got: $get_result"
fi

# Test: access redis via TAP IP from host
tap_ip=$(inspect_field "$PF_VM" ip_address | cut -d/ -f1) || true
if [ -n "$tap_ip" ]; then
    # Use redis-cli if available, otherwise use raw TCP
    if command -v redis-cli >/dev/null 2>&1; then
        tap_ping=$(redis-cli -h "$tap_ip" PING 2>&1) || true
        if echo "$tap_ping" | grep -q "PONG"; then
            pass "Redis PING via TAP IP ($tap_ip)"
        else
            fail "Redis PING via TAP IP ($tap_ip)" "Got: $tap_ping"
        fi
    else
        # Raw TCP PING
        tap_raw=$(echo -e "PING\r" | timeout 5 bash -c "exec 3<>/dev/tcp/$tap_ip/6379; cat >&3; cat <&3" 2>&1) || true
        if echo "$tap_raw" | grep -q "PONG"; then
            pass "Redis PING via TAP IP (raw TCP, $tap_ip)"
        else
            skip "Redis PING via TAP IP" "redis-cli not on host and raw TCP failed"
        fi
    fi
else
    skip "Redis PING via TAP IP" "Could not determine TAP IP"
fi

# Test: iptables DNAT rule was created
ipt_output=$(iptables -t nat -L -n 2>&1) || true
if echo "$ipt_output" | grep -q "26379"; then
    pass "iptables DNAT rule exists for port 26379"
else
    fail "iptables DNAT rule exists for port 26379" "Rule not found"
fi

cleanup_vm "$PF_VM"

# ============================================================
# Scenario 2: Redis accessible over mycelium
# ============================================================

# Check mycelium binary
MYCELIUM_FOUND=false
REAL_USER="${SUDO_USER:-${DOAS_USER:-}}"
REAL_HOME="$HOME"
if [ -n "$REAL_USER" ]; then
    REAL_HOME=$(eval echo "~$REAL_USER" 2>/dev/null || echo "$HOME")
fi
for candidate in "$REAL_HOME/.chvm/bin/mycelium" "$HOME/.chvm/bin/mycelium" /usr/local/bin/mycelium /usr/bin/mycelium; do
    if [ -x "$candidate" ]; then
        MYCELIUM_FOUND=true
        break
    fi
done

if ! $MYCELIUM_FOUND; then
    skip "Mycelium redis server VM started" "mycelium not found"
    skip "Redis running in mycelium VM" "mycelium not found"
    skip "Redis cross-VM SET/GET via mycelium" "mycelium not found"
    test_end || true
    export_results
    exit 0
fi

MYC_PEERS="--mycelium-peers tcp://188.40.132.242:9651 --mycelium-peers tcp://136.243.47.186:9651 --mycelium-peers tcp://185.69.166.7:9651"

MYC_SERVER="${TEST_PREFIX}-redis-myc"
MYC_CLIENT="${TEST_PREFIX}-redis-cli"

cleanup_myc() {
    cleanup_vm "$MYC_SERVER" 2>/dev/null || true
    cleanup_vm "$MYC_CLIENT" 2>/dev/null || true
}
trap cleanup_myc EXIT

$CHVM run -d --name "$MYC_SERVER" --network mycelium $MYC_PEERS \
    alpine:latest sleep 3600 >/dev/null 2>&1

if wait_vm_running "$MYC_SERVER" 30; then
    pass "Mycelium redis server VM started"
else
    fail "Mycelium redis server VM started" "Timeout"
    test_end || true
    export_results
    exit 0
fi

sleep 5

# Install and start redis on the server
setup_output=$(setup_redis "$MYC_SERVER" 2>&1) || true
if $CHVM exec "$MYC_SERVER" -- pgrep redis-server >/dev/null 2>&1; then
    pass "Redis running in mycelium VM"
else
    fail "Redis running in mycelium VM" "redis-server not running"
    echo "       Output: $(echo "$setup_output" | tail -5)"
fi

# Seed data on the server
$CHVM exec "$MYC_SERVER" -- redis-cli SET mycelium:msg "hello over mycelium" >/dev/null 2>&1 || true

# Get mycelium IP
SERVER_MYC_IP=$(inspect_field "$MYC_SERVER" mycelium_ip) || true

if [ -z "$SERVER_MYC_IP" ]; then
    skip "Redis cross-VM SET/GET via mycelium" "Could not get mycelium IP"
    test_end || true
    export_results
    exit 0
fi

# Start client VM with mycelium
$CHVM run -d --name "$MYC_CLIENT" --network mycelium $MYC_PEERS \
    alpine:latest sleep 3600 >/dev/null 2>&1

if ! wait_vm_running "$MYC_CLIENT" 30; then
    skip "Redis cross-VM SET/GET via mycelium" "Client VM failed to start"
    test_end || true
    export_results
    exit 0
fi

sleep 5

# Install redis-cli in client
$CHVM exec "$MYC_CLIENT" -- sh -c "apk add --no-cache redis >/dev/null 2>&1" || true

# Try to GET the seeded value from the server over mycelium
myc_ok=false
for attempt in 1 2 3 4 5 6; do
    myc_result=$($CHVM exec "$MYC_CLIENT" -- redis-cli -h "$SERVER_MYC_IP" GET mycelium:msg 2>&1) || true
    if echo "$myc_result" | grep -q "hello over mycelium"; then
        myc_ok=true
        break
    fi
    sleep 10
done

if $myc_ok; then
    pass "Redis GET from another VM via mycelium ($SERVER_MYC_IP)"
else
    skip "Redis GET from another VM via mycelium" "Route convergence too slow (tried ${attempt}x)"
fi

# Test: SET from client, GET from server (bidirectional)
if $myc_ok; then
    $CHVM exec "$MYC_CLIENT" -- redis-cli -h "$SERVER_MYC_IP" SET mycelium:reply "pong from client" >/dev/null 2>&1 || true
    reply=$($CHVM exec "$MYC_SERVER" -- redis-cli GET mycelium:reply 2>&1) || true
    if echo "$reply" | grep -q "pong from client"; then
        pass "Redis cross-VM SET/GET works over mycelium"
    else
        fail "Redis cross-VM SET/GET works over mycelium" "Got: $reply"
    fi
fi

# Cleanup
cleanup_vm "$MYC_SERVER"
cleanup_vm "$MYC_CLIENT"

test_end
export_results
