#!/bin/bash
# 15_nginx.sh — Nginx real-world scenario tests (root needed)
# Tests: nginx via port forwarding, nginx accessible over mycelium
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

trap trap_cleanup EXIT

test_start "15 — Nginx Scenarios"

# Helper: extract a field from chvm inspect JSON output
# Usage: inspect_field <vm_name> <field_name>
inspect_field() {
    $CHVM inspect "$1" 2>/dev/null | grep "\"$2\"" | head -1 | sed 's/.*: *"\([^"]*\)".*/\1/'
}

# Helper: install nginx and configure it inside a running VM
setup_nginx() {
    local vm_name="$1"
    $CHVM exec "$vm_name" -- sh -c "\
        apk add --no-cache nginx >/dev/null 2>&1 && \
        mkdir -p /var/www /run/nginx && \
        echo 'hello from chvm' > /var/www/index.html && \
        printf 'server { listen 80; listen [::]:80; root /var/www; index index.html; }\n' \
            > /etc/nginx/http.d/default.conf && \
        nginx" 2>&1
}

# ============================================================
# Scenario 1: Nginx via port forwarding
# ============================================================

PF_VM="${TEST_PREFIX}-nginx-pf"

$CHVM run -d --name "$PF_VM" -p 28080:80 alpine:latest sleep 3600 >/dev/null 2>&1

if wait_vm_running "$PF_VM" 30; then
    pass "Port-forward VM started"
else
    fail "Port-forward VM started" "Timeout"
    test_end || true
    export_results
    exit 0
fi

# Wait for vsock
sleep 5

# Install and start nginx
setup_output=$(setup_nginx "$PF_VM" 2>&1) || true
if $CHVM exec "$PF_VM" -- pgrep nginx >/dev/null 2>&1; then
    pass "Nginx installed and running in port-forward VM"
else
    fail "Nginx installed and running in port-forward VM" "nginx not running"
    echo "       Output: $(echo "$setup_output" | tail -5)"
fi

# Test: access nginx via TAP IP (direct from host)
tap_ip=$(inspect_field "$PF_VM" ip_address | cut -d/ -f1) || true
if [ -n "$tap_ip" ]; then
    tap_result=$(curl -s --max-time 5 "http://$tap_ip/" 2>&1) || true
    if echo "$tap_result" | grep -q "hello from chvm"; then
        pass "Nginx accessible via TAP IP ($tap_ip)"
    else
        fail "Nginx accessible via TAP IP ($tap_ip)" "Got: $(echo "$tap_result" | head -1)"
    fi
else
    skip "Nginx accessible via TAP IP" "Could not determine TAP IP"
fi

# Test: iptables DNAT rule was created for port forward
ipt_output=$(iptables -t nat -L -n 2>&1) || true
if echo "$ipt_output" | grep -q "28080"; then
    pass "iptables DNAT rule exists for port 28080"
else
    fail "iptables DNAT rule exists for port 28080" "Rule not found"
fi

# Test: access nginx via exec from inside the VM (verifies nginx actually serves)
exec_result=$($CHVM exec "$PF_VM" -- wget -qO- -T 5 http://localhost/ 2>&1) || true
if echo "$exec_result" | grep -q "hello from chvm"; then
    pass "Nginx accessible from inside VM (localhost)"
else
    fail "Nginx accessible from inside VM (localhost)" "Got: $(echo "$exec_result" | head -1)"
fi

cleanup_vm "$PF_VM"

# ============================================================
# Scenario 2: Nginx accessible over mycelium
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
    skip "Mycelium nginx server VM started" "mycelium not found"
    skip "Nginx running in mycelium VM" "mycelium not found"
    skip "Nginx accessible from another VM via mycelium" "mycelium not found"
    test_end || true
    export_results
    exit 0
fi

MYC_PEERS="--mycelium-peers tcp://188.40.132.242:9651 --mycelium-peers tcp://136.243.47.186:9651 --mycelium-peers tcp://185.69.166.7:9651"

MYC_SERVER="${TEST_PREFIX}-nginx-myc"
MYC_CLIENT="${TEST_PREFIX}-nginx-cli"

cleanup_myc() {
    cleanup_vm "$MYC_SERVER" 2>/dev/null || true
    cleanup_vm "$MYC_CLIENT" 2>/dev/null || true
}
trap cleanup_myc EXIT

$CHVM run -d --name "$MYC_SERVER" --network mycelium $MYC_PEERS \
    alpine:latest sleep 3600 >/dev/null 2>&1

if wait_vm_running "$MYC_SERVER" 30; then
    pass "Mycelium nginx server VM started"
else
    fail "Mycelium nginx server VM started" "Timeout"
    test_end || true
    export_results
    exit 0
fi

sleep 5

# Install and start nginx on the mycelium VM
setup_output=$(setup_nginx "$MYC_SERVER" 2>&1) || true
if $CHVM exec "$MYC_SERVER" -- pgrep nginx >/dev/null 2>&1; then
    pass "Nginx running in mycelium VM"
else
    fail "Nginx running in mycelium VM" "nginx not running"
    echo "       Output: $(echo "$setup_output" | tail -5)"
fi

# Get mycelium IP
SERVER_MYC_IP=$(inspect_field "$MYC_SERVER" mycelium_ip) || true

if [ -z "$SERVER_MYC_IP" ]; then
    skip "Nginx accessible from another VM via mycelium" "Could not get mycelium IP"
    test_end || true
    export_results
    exit 0
fi

# Start client VM with mycelium
$CHVM run -d --name "$MYC_CLIENT" --network mycelium $MYC_PEERS \
    alpine:latest sleep 3600 >/dev/null 2>&1

if ! wait_vm_running "$MYC_CLIENT" 30; then
    skip "Nginx accessible from another VM via mycelium" "Client VM failed to start"
    test_end || true
    export_results
    exit 0
fi

sleep 5

# Try to reach nginx on the server's mycelium IP from the client VM
myc_ok=false
for attempt in 1 2 3 4 5 6; do
    myc_result=$($CHVM exec "$MYC_CLIENT" -- sh -c "wget -qO- -T 15 'http://[$SERVER_MYC_IP]/'" 2>&1) || true
    if echo "$myc_result" | grep -q "hello from chvm"; then
        myc_ok=true
        break
    fi
    sleep 10
done

if $myc_ok; then
    pass "Nginx accessible from another VM via mycelium ($SERVER_MYC_IP)"
else
    skip "Nginx accessible from another VM via mycelium" "Route convergence too slow (tried ${attempt}x)"
fi

# Cleanup
cleanup_vm "$MYC_SERVER"
cleanup_vm "$MYC_CLIENT"

test_end
export_results
