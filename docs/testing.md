# Testing

## Overview

chvm uses a shell-based integration test suite that exercises the full CLI and VM lifecycle end-to-end. Tests run against real Cloud Hypervisor VMs using real OCI images.

The test suite consists of 14 numbered suites covering build verification, CLI parsing, image management, VM lifecycle, exec/attach, networking, storage, kernel management, mycelium, and rootfs/storage backends. The suite uses a lightweight bash test harness with no external test framework dependencies.

## Running Tests

### Full Suite

```bash
# Run all tests (requires root for VM operations)
sudo bash tests/run_tests.sh
```

The runner:
1. Builds the workspace in release mode
2. Builds chvm-init with musl target
3. Runs each numbered suite in order
4. Aggregates and displays final results
5. Cleans up any leftover test VMs

### Individual Suites

```bash
# Run a specific suite
sudo bash tests/04_vm_lifecycle.sh

# Run the expect-based attach test
sudo expect tests/07_attach.expect test-attach-vm
```

### Expected Results

```
Total: 79 tests — 75 pass, 0 fail, 4 skip
ALL TESTS PASSED
```

The 4 skips are expected:
- `cargo build` -- skipped when running as root without cargo
- `kernel set-default` -- requires multiple kernels
- `mycelium ping` (x2) -- route convergence too slow in CI

With the custom kernel (`vmlinux-chvm-6.6.70`), all storage and log tests pass. With the host-extracted kernel, the virtiofs test may skip (kernel lacks `CONFIG_VIRTIO_FS`) and the logs test may skip (empty console output).

## Test Structure

```
tests/
  run_tests.sh          # Main test runner
  helpers.sh            # Shared test harness
  01_build.sh           # Build verification (no root)
  02_cli_basics.sh      # CLI parsing (no root)
  03_image_management.sh # OCI pull & cache (root)
  04_vm_lifecycle.sh    # VM lifecycle (root)
  05_vm_inspection.sh   # List, logs, inspect (root)
  06_exec.sh            # Exec via vsock (root)
  07_attach.expect      # Interactive attach (root, expect)
  08_networking.sh      # TAP networking (root)
  09_port_forwarding.sh # Port forwards (root)
  10_resources.sh       # CPU, memory, env (root)
  11_storage.sh         # Volume mounts (root)
  12_kernel.sh          # Kernel management (partial root)
  13_mycelium.sh        # Mycelium networking (root, internet)
  14_rootfs.sh          # Rootfs and storage backends (root)
```

Suites are numbered for execution order. Later suites depend on images pulled by earlier ones (e.g., suite 03 pulls alpine:latest, which suites 04-11 use).

## Test Harness

### helpers.sh

The shared harness provides:

**Assertion functions:**

| Function | Purpose |
|----------|---------|
| `assert_success "$desc" "$cmd"` | Run command, expect exit 0 |
| `assert_failure "$desc" "$cmd"` | Run command, expect non-zero exit |
| `assert_output "$desc" "$cmd" "$expected"` | Run command, check stdout contains substring |
| `assert_exit "$desc" "$cmd" "$code"` | Run command, check specific exit code |
| `pass "$desc"` | Record a passing test |
| `fail "$desc"` | Record a failing test |
| `skip "$desc" "$reason"` | Record a skipped test |

**VM helpers:**

| Function | Purpose |
|----------|---------|
| `wait_vm_running "$name" "$timeout"` | Poll `chvm list` until VM is running |
| `wait_vm_stopped "$name" "$timeout"` | Poll `chvm list --all` until VM is stopped |
| `cleanup_vm "$name"` | Force stop + rm, ignore errors |
| `cleanup_all` | Remove all test VMs matching `$TEST_PREFIX` |

**Reporting:**

| Function | Purpose |
|----------|---------|
| `test_start "$suite"` | Print suite header, reset counters |
| `test_end` | Print pass/fail/skip summary |
| `export_results` | Write results to `$RESULTS_FILE` for aggregation |

### Test naming

Each test VM is named with a unique prefix `test_<PID>` to avoid collision with user VMs. The prefix is generated from the runner's PID and exported as `$TEST_PREFIX`.

### Cleanup

Each suite installs a trap handler that calls `cleanup_all` on exit (including errors). The main runner also performs a final sweep to remove any test VMs that weren't cleaned up.

## Suite Details

### 01_build.sh -- Build Verification

Tests that don't require root:
- `cargo build --workspace` succeeds (skipped when cargo unavailable)
- Binary exists and is executable
- `chvm --help` exits 0 and mentions "Cloud Hypervisor"
- `chvm version` contains version string

### 02_cli_basics.sh -- CLI Parsing

Tests CLI help output and basic commands:
- `--help` lists all expected subcommands
- `run --help` shows all expected flags
- `chvm doctor` exits 0
- `chvm list` and `chvm images` exit 0 (empty tables)
- Invalid subcommand exits non-zero
- Command aliases (`ps`, `ls`) work

### 03_image_management.sh -- OCI Pull & Cache

Tests image pulling from Docker Hub:
- Pull busybox:latest and alpine:latest
- `chvm images` lists both images
- Re-pulling same image is fast (cache hit)
- Pulling non-existent image fails with error

### 04_vm_lifecycle.sh -- Core Lifecycle

The largest suite, testing the full VM lifecycle:
- Run foreground with `--rm` and verify output
- Run detached, verify Running status
- Stop running VM, verify Stopped status
- Remove stopped VM, verify removed
- Create + Start workflow
- Force remove running VM
- `--rm` auto-cleanup after exit
- Error cases: stop already-stopped, rm non-existent
- Prune removes all stopped VMs

### 05_vm_inspection.sh -- List, Logs, Inspect

Tests VM introspection commands:
- `chvm list` shows running VMs
- `chvm list --all` includes stopped VMs
- `chvm inspect` outputs expected JSON fields
- `chvm logs` contains boot messages
- Status filtering (running vs all)

### 06_exec.sh -- Command Execution

Tests exec via vsock:
- `exec echo hello` returns correct output
- `exec ls /` contains expected directories
- `-e FOO=bar` passes environment variables
- `-w /tmp` sets working directory
- `exec false` returns non-zero exit code

### 07_attach.expect -- Interactive Attach

TCL expect script that tests the interactive shell:
- Starts a detached VM
- Attaches and waits for shell prompt
- Sends `echo EXPECT_TEST_OK` and verifies response
- Sends `ls /` and verifies output
- Exits cleanly

### 08_networking.sh -- TAP Networking

Tests network configuration inside the VM:
- VM eth0 has an IP address in 192.168.200.0/24
- Gateway reachability (may skip depending on host config)
- Internet access via NAT (may skip)
- DNS resolution (may skip)

### 09_port_forwarding.sh -- Port Forwards

Tests port forwarding setup:
- VM with `-p 18080:8080` starts successfully
- iptables DNAT rule exists for the port
- `chvm inspect` shows port forward configuration

### 10_resources.sh -- Resource Configuration

Tests CPU, memory, and environment settings:
- `--memory 256` allocates approximately 256MB
- `-e TESTVAR=hello123` passes environment variable
- Multiple `-e` flags all take effect
- `--cpus 2` creates VM with 2 vCPUs

### 11_storage.sh -- Volume Mounts

Tests virtiofs volume mounts:
- Mounted directory contents are visible in guest
- File contents are readable
- Nested directories are accessible

### 12_kernel.sh -- Kernel Management

Tests kernel management commands:
- `chvm kernel list` exits 0
- `chvm kernel check` shows capability output
- `chvm kernel extract-host` extracts the host kernel
- `chvm kernel set-default` creates default symlink

### 13_mycelium.sh -- Mycelium Networking

Tests mycelium peer-to-peer IPv6 overlay networking:
- Two VMs started with `--network mycelium --mycelium-peers <public_peer>`
- Both VMs have a `mycelium` TUN interface
- Both VMs assigned unique mycelium IPv6 addresses
- Both VMs connected to the public peer (TCP ESTABLISHED)
- VM-to-VM ping via mycelium IPv6 (may skip — route convergence takes ~60s)
- Mycelium IP visible in `chvm inspect` output

Requires: mycelium binary at `~/.chvm/bin/mycelium` and internet access to reach the public peer. Skipped entirely if mycelium binary is not found.

### 14_rootfs.sh -- Rootfs & Storage Backends

Tests local rootfs and storage backend options:
- Run a command using `--rootfs` with a cached OCI rootfs directory
- Verify filesystem structure from local rootfs
- Run with `--storage virtiofs` backend (requires virtiofsd + kernel with virtiofs support; skips gracefully if either is missing)

## Unit Tests

Three modules include inline Rust unit tests:

**`hypervisor/cmdline.rs`:**
- Tests basic cmdline building
- Tests base64 command encoding

**`oci/registry.rs`:**
- Tests reference normalization (simple names, tags, user/repo, full references)

**`vm/state.rs`:**
- Tests `NetworkMode` serialization, deserialization, Display, and FromStr

Run with:

```bash
cargo test --workspace
```

## Writing New Tests

To add a new test suite:

1. Create `tests/NN_name.sh` with the next number
2. Source the helpers:
   ```bash
   #!/bin/bash
   set -euo pipefail
   SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
   PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"
   cd "$PROJECT_DIR"
   source "$SCRIPT_DIR/helpers.sh"
   ```
3. Start the suite:
   ```bash
   test_start "NN -- Suite Name"
   ```
4. Write tests using assertion functions:
   ```bash
   assert_success "description" "$CHVM some-command"
   assert_output "description" "$CHVM some-command" "expected output"
   ```
5. End and export:
   ```bash
   test_end
   export_results
   ```

The runner automatically picks up new `[0-9]*.sh` and `[0-9]*.expect` files.
