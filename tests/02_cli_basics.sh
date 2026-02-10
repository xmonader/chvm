#!/bin/bash
# 02_cli_basics.sh — CLI parsing tests (no root needed)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
cd "$PROJECT_DIR"
source "$SCRIPT_DIR/helpers.sh"

test_start "02 — CLI Basics"

# 1. --help lists all expected subcommands
help_output=$($CHVM --help 2>&1)
all_found=true
for subcmd in pull run create start stop rm exec attach list logs inspect images prune doctor setup kernel version; do
    if ! echo "$help_output" | grep -qw "$subcmd"; then
        fail "--help lists subcommand '$subcmd'" "Not found in help output"
        all_found=false
    fi
done
if $all_found; then
    pass "--help lists all expected subcommands"
fi

# 2. chvm run --help shows all flags
run_help=$($CHVM run --help 2>&1)
flags_found=true
for flag in "--name" "--cpus" "--memory" "--detach" "--rm" "--publish" "--env" "--volume"; do
    if ! echo "$run_help" | grep -qF -- "$flag"; then
        fail "run --help shows flag '$flag'" "Not found"
        flags_found=false
    fi
done
if $flags_found; then
    pass "run --help shows all expected flags"
fi

# 3. chvm doctor exits 0
assert_success \
    "chvm doctor exits 0" \
    "$CHVM doctor"

# 4. chvm list exits 0 (may be empty)
assert_success \
    "chvm list exits 0" \
    "$CHVM list"

# 5. chvm images exits 0 (may be empty)
assert_success \
    "chvm images exits 0" \
    "$CHVM images"

# 6. Invalid subcommand exits non-zero
assert_failure \
    "Invalid subcommand exits non-zero" \
    "$CHVM notarealcommand"

# 7. chvm version contains "chvm"
assert_output \
    "chvm version contains 'chvm'" \
    "$CHVM version" \
    "chvm"

# 8. List aliases: ps and ls
assert_success \
    "chvm ps alias works" \
    "$CHVM ps"

assert_success \
    "chvm ls alias works" \
    "$CHVM ls"

test_end
export_results
