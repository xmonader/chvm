# Architecture

## Overview

chvm is a MicroVM manager that bridges the gap between containers and full virtual machines. It provides a Docker-like CLI for running OCI container images inside Cloud Hypervisor microVMs, combining container usability with hardware-level VM isolation.

```
User CLI (chvm run alpine:latest echo hello)
  |
  v
+------------------+
|    chvm-cli      |  Command parsing, output formatting
+--------+---------+
         |
+--------v---------+
|    chvm-lib      |  Core orchestration
|  +-------------+ |
|  | VM Manager  | |  Lifecycle: create, start, stop, rm
|  +-------------+ |
|  | OCI Pull    | |  Image registry, cache, layer extraction
|  +-------------+ |
|  | Network     | |  TAP/bridge, NAT, port forwarding
|  +-------------+ |
|  | Storage     | |  Block device, virtiofs, volume mounts
|  +-------------+ |
|  | Hypervisor  | |  Cloud Hypervisor process + REST API
|  +-------------+ |
|  | Vsock       | |  Host-guest communication proxy
|  +-------------+ |
+--------+---------+
         |
   Cloud Hypervisor (VMM)
         |
+--------v---------+
|   Guest VM       |
|  +-------------+ |
|  | chvm-init   | |  PID 1: boot, network, modules, vsock
|  +-------------+ |
|  | User Process| |  The command the user asked to run
|  +-------------+ |
+------------------+
```

## Workspace Structure

The project is a Cargo workspace with three crates:

```
chvm/
  Cargo.toml              # Workspace root
  crates/
    chvm-cli/             # Binary crate - CLI frontend
    chvm-lib/             # Library crate - all core logic
    chvm-init/            # Binary crate - guest init process
  tests/                  # Integration test suite (shell scripts)
  docs/                   # Documentation
```

### chvm-cli (~1,000 lines)

The CLI binary. Uses clap derive macros for argument parsing, then delegates to `chvm-lib` for all operations. Handles output formatting (tables, JSON, colors) and terminal interaction (attach mode).

```
chvm-cli/src/
  main.rs           # Entry point, dispatch to command handlers
  cli.rs            # Clap definitions for all 16 commands
  commands/
    mod.rs          # Module re-exports, shared utilities (truncate, build_create_options)
    run.rs          # Create+start, foreground/detached modes
    create.rs       # Create VM without starting
    start.rs        # Start a created/stopped VM
    stop.rs         # Stop a running VM
    rm.rs           # Remove a VM
    exec.rs         # Execute command in running VM
    attach.rs       # Interactive shell in running VM
    pull.rs         # Pull OCI image
    list.rs         # List VMs (aliases: ps, ls)
    logs.rs         # Show console logs
    inspect.rs      # Show VM state as JSON
    images.rs       # List cached images
    prune.rs        # Remove all stopped VMs
    doctor.rs       # System requirements check
    setup.rs        # Setup kernel, network
    kernel.rs       # Kernel management subcommands
    version.rs      # Version info
```

### chvm-lib (~4,000 lines)

The core library. All VM management logic, OCI operations, networking, storage, and hypervisor integration. No CLI concerns -- pure library code.

```
chvm-lib/src/
  lib.rs            # Module declarations
  config.rs         # Global Config (from ~/.chvm/config.json)
  error.rs          # ChvmError enum with thiserror
  paths.rs          # Directory layout (~/.chvm/*)
  utils.rs          # Shared utilities (user home resolution)
  vm/
    state.rs        # All state/config structs (VmConfig, RuntimeState, NetworkMode enum, etc.)
    builder.rs      # VmBuilder: merges CLI args + image config + defaults
    manager.rs      # VmManager: create, start, stop, rm, list, exec, attach
  hypervisor/
    api.rs          # Cloud Hypervisor REST API client (Unix socket)
    cmdline.rs      # Kernel command line builder with base64 encoding
    process.rs      # CH process launch, discovery, lifecycle
  kernel/
    manager.rs      # Kernel selection, extraction, capability detection
  network/
    traits.rs       # NetworkBackend trait
    bridge.rs       # Linux bridge + NAT setup
    tap.rs          # TAP device backend
    mycelium.rs     # Mycelium overlay network backend
    namespace.rs    # Network namespace helpers
  oci/
    pull.rs         # OCI image pull from registry
    cache.rs        # Image cache with index
    registry.rs     # Reference normalization
    layer.rs        # Tar layer extraction with whiteout handling
    image_config.rs # OCI image config parsing
  storage/
    traits.rs       # StorageBackend trait
    block.rs        # Virtio-block backend
    virtiofs.rs     # Virtio-FS backend with virtiofsd
    image.rs        # ext4 image creation, init/module injection
  vsock/
    protocol.rs     # Vsock protocol constants and message types
    proxy.rs        # Host-side vsock proxy for exec/attach
```

### chvm-init (~950 lines)

The guest init process. Compiled statically with musl (`x86_64-unknown-linux-musl`) so it works inside any Linux guest regardless of libc. Runs as PID 1 and bootstraps the guest environment.

```
chvm-init/src/
  main.rs            # Boot sequence orchestration
  cmdline_parser.rs  # Parse base64-encoded config from /proc/cmdline
  devices.rs         # Create /dev nodes (null, zero, random, etc.)
  exec_handler.rs    # Handle exec requests over vsock
  helpers.rs         # Shared utilities (binary lookup, env setup)
  mounts.rs          # Mount proc, sys, dev, tmpfs, virtiofs volumes
  mycelium.rs        # Start mycelium daemon in guest
  network.rs         # Configure eth0, IP, gateway, DNS
  rootfs.rs          # Mount virtiofs rootfs + pivot_root
  shell_handler.rs   # PTY-based interactive shell over vsock
  signal.rs          # SIGCHLD-based zombie reaping, poweroff/reboot
  vsock_server.rs    # AF_VSOCK listener on ports 1024/1025
```

## Data Flow

### VM Creation

```
chvm run -d --name myvm -e FOO=bar -p 8080:80 alpine:latest sleep 3600
```

1. **Image resolution**: Check `ImageCache` for "alpine:latest". If not cached, pull from Docker Hub -- normalize reference, resolve multi-arch manifest (linux/amd64), stream layers to disk (avoiding buffering entire layers in memory).

2. **Rootfs extraction**: Extract OCI layers in order to `~/.chvm/images/<digest>/rootfs/`, handling whiteout files for layer deletions.

3. **VM configuration**: `VmBuilder` merges CLI arguments with OCI image config (entrypoint, cmd, env, workdir) and global defaults. Generates a UUID-based VM ID, random MAC address, and vsock CID (FNV-1a hash of VM ID across the full 32-bit CID space).

4. **Kernel cmdline**: Encode the command (`sleep 3600`), environment (`FOO=bar`), and working directory as base64 in the kernel command line parameters, readable by `chvm-init` inside the guest.

5. **State persistence**: Write `RuntimeState` as JSON to `~/.chvm/vms/<id>/state.json` with status `Created`.

### VM Start

1. **Storage preparation**: Create a sparse ext4 image from the rootfs directory. Copy all files (skipping symlink loops), inject the `chvm-init` binary and kernel modules (vsock, virtiofs). When mycelium networking is enabled, the `mycelium` binary is also injected into the guest at `/usr/bin/mycelium`. Mount operations use RAII guards to ensure cleanup on failure.

2. **Network setup**: Create a TAP device, attach to the `chvm0` bridge, dynamically allocate an unused IP from 192.168.200.0/24 (scanning existing bridge devices to avoid collisions), configure iptables DNAT rules for port forwards, enable NAT masquerade. On teardown, iptables rules are cleaned up.

3. **Volume preparation**: For each `-v` mount, launch a separate `virtiofsd` instance with a unique tag.

4. **Cloud Hypervisor launch**: Assemble CH arguments and spawn the process. Wait for the API socket to become available (up to 30s).

5. **State update**: Record PID, update status to `Running`.

6. **Mycelium IP probe** (if mycelium enabled): After boot, exec `ip -6 addr show dev mycelium` inside the guest (with retries) to discover the mycelium IPv6 address and store it in `network_state.mycelium_ip`.

### Inside the Guest (chvm-init boot)

1. Mount essential filesystems (proc, sys, dev, tmpfs)
2. Create device nodes (/dev/null, /dev/zero, etc.)
3. Mount rootfs (virtiofs mode) or detect block device mount
4. Configure network (bring up eth0, set IP/gateway, write resolv.conf)
5. Load kernel modules (vsock, virtiofs) via `init_module` syscall
6. Mount any virtiofs volumes
7. Start mycelium daemon if configured
8. Start vsock servers for exec (port 1024) and shell (port 1025)
9. Parse and execute the user's command
10. On exit, power off the VM

### Exec Path

```
chvm exec myvm ls /
  |
  v
Host proxy connects to VM's vsock Unix socket
  |
  v
Sends "CONNECT 1024\n" to CH vsock proxy
  |
  v
CH forwards to guest AF_VSOCK port 1024
  |
  v
Guest exec_handler receives JSON ExecRequest
  |
  v
Spawns process, reads stdout/stderr concurrently (separate threads)
  |
  v
Streams OUTPUT: lines back, then ExecResponse JSON with exit code
```

### Attach Path

Similar to exec but uses port 1025. The guest `shell_handler` allocates a PTY, spawns a shell (ash/bash/sh), and bidirectionally proxies between the vsock socket and the PTY master fd. The host proxy bridges stdin/stdout with the vsock stream using raw mode terminal.

## State Machine

```
        create
          |
          v
      +---------+
      | Created |
      +----+----+
           |
       start
           |
           v
      +---------+     stop      +---------+    start     +---------+
      | Running | ------------> | Stopped | -----------> | Running |
      +----+----+               +----+----+              +---------+
           |                         |
           | crash/exit              |
           v                         |
      +---------+                    |
      | Failed  |                    |
      +---------+                    |
                                     |
       rm -f (any state)            rm
       ------------------------------>  (deleted from disk)
```

States (from `VmState` enum):
- `Created` -- VM config written, not yet started
- `Running` -- Cloud Hypervisor process is alive
- `Stopped` -- VM was stopped gracefully or the CH process exited
- `Failed` -- VM encountered an error during start

Transitions:
- `Created -> Running`: `chvm start` or `chvm run`
- `Running -> Stopped`: `chvm stop`, process exit, or VM crash
- `Stopped -> Running`: `chvm start` (restart)
- Any state -> deleted: `chvm rm` (stopped) or `chvm rm -f` (force, any state)

There is no "Removed" state -- `chvm rm` deletes the VM's state directory from disk entirely.

## State Persistence

Each VM's state is stored as a JSON file:

```
~/.chvm/vms/<vm-id>/
  state.json          # RuntimeState (config, status, PIDs, timestamps)
  rootfs.img          # ext4 block device image
  api.sock            # Cloud Hypervisor API socket
  vsock.sock          # Cloud Hypervisor vsock proxy socket
  console.log         # Serial console output
  cloud-hypervisor.log
```

The `StateStore` in `VmManager` scans all VM directories on startup, loads their `state.json` files, and reconciles with actual process state (checking if PIDs are still alive).

## Key Design Decisions

### Kernel cmdline as config transport

Guest configuration (command, environment, working directory, network config, mount info) is encoded in the kernel command line as base64 parameters. This avoids needing a secondary config channel -- the guest init reads `/proc/cmdline` immediately on boot.

### vsock for host-guest communication

Instead of network-based protocols, chvm uses virtio-vsock (a VM-to-host socket transport) for exec and attach operations. This works regardless of network configuration and provides a direct, low-latency channel.

### Custom kernel with built-in drivers

chvm includes a build script (`scripts/build-kernel.sh`) that produces a minimal ~23MB kernel with `CONFIG_MODULES=n`. All required drivers (vsock, virtiofs, TUN, IPv6, virtio-PCI) are compiled directly into the kernel. This eliminates the need to inject kernel modules into the rootfs image and removes the `load_kernel_modules()` step during guest boot, resulting in faster startup and a simpler boot path.

### Block device as default storage

The default storage backend creates an ext4 disk image from the OCI rootfs. This is simpler than virtiofs (no daemon needed), provides better isolation, and works with any kernel. The kernel boots with `root=/dev/vda` and Cloud Hypervisor presents the image as a virtio-block device. When using the host kernel, the chvm-init binary and required kernel modules are injected directly into the image. When using a custom-built kernel (with all drivers built-in), module injection is automatically skipped.

### VirtioFS as alternative storage

The virtiofs backend creates an overlay filesystem over the cached rootfs and serves it via virtiofsd. The kernel boots with `root=rootfs rootfstype=virtiofs`, mounting the virtiofs share directly as root. This requires a kernel with `CONFIG_FUSE_FS=y` and `CONFIG_VIRTIO_FS=y` — the custom kernel has these built-in, while the host-extracted kernel typically does not. Like the block backend, `chvm-init` is injected into the overlay before virtiofsd starts.

### Static musl init binary

The `chvm-init` binary is compiled with the `x86_64-unknown-linux-musl` target for static linking. This ensures it works inside any Linux guest rootfs, regardless of whether the guest uses glibc, musl, or has no libc at all.

### Trait-based backends

Network and storage use trait-based backends (`NetworkBackend`, `StorageBackend`), allowing clean separation between TAP/mycelium networking and block/virtiofs storage without conditional logic in the manager.
