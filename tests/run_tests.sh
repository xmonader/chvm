#!/bin/bash
# run_tests.sh — Main test runner for CHVM automation test suite
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"

# Ensure cargo is in PATH (needed when running via doas)
export PATH="$HOME/.cargo/bin:$PATH:/home/xmonader/.cargo/bin:/usr/local/bin"

TOTAL_PASS=0
TOTAL_FAIL=0
TOTAL_SKIP=0
SUITE_RESULTS=()

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[0;33m'
BLUE='\033[0;34m'
BOLD='\033[1m'
NC='\033[0m'

export TEST_PREFIX="test_$$"

echo -e "${BOLD}${BLUE}===============================${NC}"
echo -e "${BOLD}${BLUE}  CHVM Automation Test Suite${NC}"
echo -e "${BOLD}${BLUE}===============================${NC}"
echo ""
echo "Test prefix: $TEST_PREFIX"
echo "Date: $(date)"
echo ""

# Build first (skip if binaries already exist and cargo isn't available)
if cargo --version >/dev/null 2>&1; then
    echo -e "${BOLD}Building chvm...${NC}"
    if cargo build --workspace --release 2>&1 | tail -3; then
        echo -e "${GREEN}Build succeeded${NC}"
    else
        echo -e "${RED}Build failed — aborting tests${NC}"
        exit 1
    fi

    # Build chvm-init with musl for guest compatibility
    echo -e "${BOLD}Building chvm-init (musl)...${NC}"
    if cargo build -p chvm-init --release --target x86_64-unknown-linux-musl 2>&1 | tail -3; then
        echo -e "${GREEN}chvm-init (musl) build succeeded${NC}"
    else
        echo -e "${YELLOW}chvm-init (musl) build failed — guest init may not work${NC}"
    fi
else
    if [ -x "./target/release/chvm" ]; then
        echo -e "${YELLOW}cargo not available — using existing binaries${NC}"
    else
        echo -e "${RED}No cargo and no pre-built binary — aborting${NC}"
        exit 1
    fi
fi
echo ""

# Run each suite in order
for suite in "$SCRIPT_DIR"/[0-9]*.sh "$SCRIPT_DIR"/[0-9]*.expect; do
    [ -f "$suite" ] || continue
    suite_name="$(basename "$suite")"
    echo -e "${BOLD}${BLUE}>>> Running $suite_name${NC}"

    results_file=$(mktemp)
    export RESULTS_FILE="$results_file"
    suite_rc=0

    if [[ "$suite" == *.expect ]]; then
        if command -v expect >/dev/null 2>&1; then
            expect "$suite" "${TEST_PREFIX}-attach" || suite_rc=$?
        else
            echo -e "  ${YELLOW}SKIP${NC} expect not installed"
            echo "0 0 1" > "$results_file"
        fi
    else
        bash "$suite" || suite_rc=$?
    fi

    # Read results
    if [ -f "$results_file" ] && [ -s "$results_file" ]; then
        read -r p f s < "$results_file"
        TOTAL_PASS=$((TOTAL_PASS + p))
        TOTAL_FAIL=$((TOTAL_FAIL + f))
        TOTAL_SKIP=$((TOTAL_SKIP + s))
        SUITE_RESULTS+=("$suite_name: ${GREEN}$p pass${NC} ${RED}$f fail${NC} ${YELLOW}$s skip${NC}")
    fi
    rm -f "$results_file"
    echo ""
done

# Final cleanup: remove any leftover test VMs
CHVM="./target/release/chvm"
[ -x "$CHVM" ] || CHVM="./target/debug/chvm"
$CHVM list --all 2>/dev/null | grep "test_" | awk '{print $2}' | xargs -r -I{} $CHVM rm -f {} 2>/dev/null || true

echo -e "${BOLD}${BLUE}===============================${NC}"
echo -e "${BOLD}${BLUE}  FINAL RESULTS${NC}"
echo -e "${BOLD}${BLUE}===============================${NC}"
echo ""
for result in "${SUITE_RESULTS[@]}"; do
    echo -e "  $result"
done
echo ""
TOTAL=$((TOTAL_PASS + TOTAL_FAIL + TOTAL_SKIP))
echo -e "${BOLD}Total: $TOTAL tests — ${GREEN}$TOTAL_PASS pass${NC}, ${RED}$TOTAL_FAIL fail${NC}, ${YELLOW}$TOTAL_SKIP skip${NC}"
echo ""

if [ "$TOTAL_FAIL" -gt 0 ]; then
    echo -e "${RED}${BOLD}SOME TESTS FAILED${NC}"
    exit 1
else
    echo -e "${GREEN}${BOLD}ALL TESTS PASSED${NC}"
    exit 0
fi
