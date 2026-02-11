# Component Design

Detailed design documentation for each subsystem in chvm.

## VM Manager

**File:** `crates/chvm-lib/src/vm/manager.rs` (~775 lines)

The VM Manager is the central orchestrator. It owns the `StateStore` (in-memory map of all VM states), delegates to storage/network backends, and manages the Cloud Hypervisor process lifecycle.

### StateStore

An in-memory `HashMap<String, RuntimeState>` that maps VM IDs to their current state. On initialization, it scans `~/.chvm/vms/*/state.json` and loads all persisted states. It reconciles with reality by checking whether recorded PIDs are still alive -- if a VM is marked `Running` but its process is dead, it's updated to `Stopped`.

### VM Identification

VMs are identified by:

- **ID**: First 12 characters of a UUID v4 (e.g., `a1b2c3d4e5f6`)
- **Name**: User-provided or defaults to the ID

Both `chvm stop myvm` (by name) and `chvm stop a1b2c3d4e5f6` (by ID) work. The manager resolves names to IDs via linear scan.

### Lifecycle Operations

| Method     | Description                                                          |
| ---------- | -------------------------------------------------------------------- |
| `create()` | Resolve image, build config, persist state as Created                |
| `start()`  | Prepare storage, setup network, launch CH, update to Running         |
| `stop()`   | API shutdown, wait/kill, teardown network+storage, update to Stopped |
| `remove()` | Stop if force, delete VM directory and state                         |
| `list()`   | Return all states, optionally filtered by running                    |
| `prune()`  | Remove all Stopped VMs                                               |
| `exec()`   | Connect to vsock port 1024, send ExecRequest, stream output          |
| `attach()` | Connect to vsock port 1025, bidirectional terminal proxy             |

---

## VM Builder

**File:** `crates/chvm-lib/src/vm/builder.rs` (~239 lines)

The builder merges configuration from three sources with clear precedence:

```
CLI arguments > OCI image config > Global defaults
```

### Configuration Merge Rules

| Field       | CLI          | Image Config      | Default                                  |
| ----------- | ------------ | ----------------- | ---------------------------------------- |
| Command     | `cmd` args   | Entrypoint + Cmd  | `/bin/sh`                                |
| Env vars    | `-e KEY=VAL` | Config.Env        | PATH, HOME, TERM                         |
| Working dir | (from image) | Config.WorkingDir | `/`                                      |
| vCPUs       | `--cpus`     |                   | `config.default_vcpu` (1)                |
| Memory      | `--memory`   |                   | `config.default_memory_mb` (512)         |
| Network     | `--network`  |                   | `config.default_network_mode` (tap)      |
| Storage     | `--storage`  |                   | `config.default_storage_backend` (block) |

### MAC Address Generation

Random MAC with OUI prefix `52:54:00` (QEMU/KVM convention):

```
52:54:00:XX:XX:XX
```

### Vsock CID Generation

Derived from the VM ID using FNV-1a hash, mapped to range 3..`u32::MAX-1` (CID 0-2 are reserved, `u32::MAX` is `VMADDR_CID_ANY`):

```rust
let mut hash: u32 = 0x811c9dc5; // FNV offset basis
for b in vm_id.bytes() {
    hash ^= b as u32;
    hash = hash.wrapping_mul(0x01000193); // FNV prime
}
let cid = (hash % (u32::MAX - 3)) + 3;
```

---

## OCI Image System

### Registry Client

**File:** `crates/chvm-lib/src/oci/pull.rs` (~86 lines)

Uses the `oci-distribution` crate for registry communication. Supports Docker Hub, GHCR, and other OCI-compliant registries. Credentials are resolved from `~/.docker/config.json` (managed via `chvm login`); anonymous access is used when no credentials are found. Layers are downloaded via streaming (not buffered in memory) to keep memory usage low for large images.

**Reference normalization** (`registry.rs`):

- `alpine` -> `docker.io/library/alpine:latest`
- `user/repo` -> `docker.io/user/repo:latest`
- `ghcr.io/user/repo:v1` -> unchanged

**Multi-architecture resolution**: Configures a platform resolver for `linux/amd64` to handle ImageIndex manifests that contain multiple architectures.

### Image Cache

**File:** `crates/chvm-lib/src/oci/cache.rs` (~188 lines)

```
~/.chvm/images/
  index.json                    # CacheIndex: digest -> CachedImage metadata
  <digest>/
    layer-0-<hash>.tar.gz      # Downloaded layer archives
    layer-1-<hash>.tar.gz
    rootfs/                     # Extracted filesystem
      bin/
      etc/
      usr/
      ...
```

The `CacheIndex` maps image digests to metadata (refs, size, layer paths, rootfs path, image config). Lookup supports both digest and reference string.

`ensure_rootfs()` extracts layers if the rootfs directory doesn't exist yet. Layers are applied in order, with OCI whiteout file handling for deletions between layers.

### Layer Extraction

**File:** `crates/chvm-lib/src/oci/layer.rs` (~108 lines)

Extracts OCI layers, detecting compression format by magic bytes (gzip: `0x1f8b`, zstd: `0x28b52ffd`, otherwise plain tar). Handles two types of OCI whiteout files:

- `.wh..wh..opq` -- Opaque whiteout: delete all contents of the parent directory before applying this layer
- `.wh.<name>` -- Delete the specific file/directory `<name>` from previous layers

Path traversal (`..`) is rejected for security.

---

## Hypervisor Integration

### Cloud Hypervisor API

**File:** `crates/chvm-lib/src/hypervisor/api.rs` (~91 lines)

Communicates with Cloud Hypervisor via its REST API over a Unix domain socket. Uses hyper 1.x with `tokio::net::UnixStream` for UDS transport.

| Endpoint                  | Method | Purpose           |
| ------------------------- | ------ | ----------------- |
| `/api/v1/vmm.ping`        | GET    | Liveness check    |
| `/api/v1/vm.info`         | GET    | VM information    |
| `/api/v1/vm.shutdown`     | PUT    | Graceful shutdown |
| `/api/v1/vm.power-button` | PUT    | ACPI power button |
| `/api/v1/vm.reboot`       | PUT    | Reboot            |

### Process Management

**File:** `crates/chvm-lib/src/hypervisor/process.rs` (~144 lines)

**Binary discovery** (in order):

1. `CHVM_CH_BIN` environment variable
2. `which cloud-hypervisor` (PATH lookup)
3. `/usr/local/bin/cloud-hypervisor`
4. `/usr/bin/cloud-hypervisor`

**Launch arguments** assembled for Cloud Hypervisor:

```
cloud-hypervisor
  --api-socket <vm_dir>/api.sock
  --cpus boot=<N>
  --memory size=<M>M,shared=on
  --kernel <kernel_path>
  --cmdline "<kernel cmdline>"
  --disk path=<rootfs.img>              # block backend
  --console file=<vm_dir>/console.log
  --net tap=<tap_device>,mac=<mac>
  --vsock cid=<cid>,socket=<vm_dir>/vsock.sock
  --fs tag=mount0,socket=<sock>         # volume mounts
  --log-file <vm_dir>/cloud-hypervisor.log
```

After spawning, the launcher polls the API socket (up to 30 seconds) until Cloud Hypervisor responds to a ping.

### Kernel Command Line

**File:** `crates/chvm-lib/src/hypervisor/cmdline.rs` (~109 lines)

Builds the kernel command line string. Uses a fluent builder pattern:

```rust
KernelCmdlineBuilder::new()
    .console("hvc0")
    .root_device("/dev/vda")
    .rw()
    .init("/chvm-init")
    .cmd(&["sleep", "3600"])     // base64-encoded
    .env(&["FOO=bar"])           // base64-encoded
    .working_dir("/app")
    .build()
```

Commands and environment variables are base64-encoded with null-byte separators to survive kernel cmdline tokenization:

```
chvm.cmd=c2xlZXAAMzYwMA==
chvm.env=Rk9PPWJhcg==
```

Additional runtime parameters are appended by the VM manager:

- `chvm.ip=192.168.200.2/24` -- Guest IP
- `chvm.gw=192.168.200.1` -- Gateway
- `chvm.mycelium_peer=tcp://...` -- Mycelium peers
- `chvm.mounts=mount0=/path,mount1=/path2` -- Volume mount mappings

---

## Network Backends

### NetworkBackend Trait

**File:** `crates/chvm-lib/src/network/traits.rs` (~41 lines)

```rust
#[async_trait]
pub trait NetworkBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn setup(&self, config: &SetupConfig) -> Result<NetworkResult>;
    async fn teardown(&self, config: &TeardownConfig) -> Result<()>;
}
```

`TeardownConfig` carries the VM ID, assigned IP address, and port forwarding rules so that the backend can clean up iptables rules and other per-VM resources on shutdown.

### TAP Backend

**File:** `crates/chvm-lib/src/network/tap.rs` (~219 lines)

Setup:

1. Create TAP device: `ip tuntap add dev tap-<id> mode tap`
2. Ensure bridge `chvm0` exists (creates if needed)
3. Attach TAP to bridge: `ip link set tap-<id> master chvm0`
4. Bring TAP up: `ip link set tap-<id> up`
5. Allocate guest IP: scan bridge devices for in-use IPs, assign first unused IP in `192.168.200.2-254/24`
6. Setup port forwarding iptables rules (DNAT + MASQUERADE)

Teardown:

1. Delete TAP device: `ip link del tap-<id>`
2. Remove port forwarding iptables rules (PREROUTING DNAT, OUTPUT DNAT, POSTROUTING MASQUERADE)

### Network Namespace Helpers

**File:** `crates/chvm-lib/src/network/namespace.rs` (~23 lines)

Utility functions for network namespace operations: `create_netns`, `delete_netns`, `create_veth`, `move_to_netns`. Used for advanced network isolation scenarios.

### Bridge Setup

**File:** `crates/chvm-lib/src/network/bridge.rs` (~73 lines)

Creates and configures the host bridge:

```bash
ip link add chvm0 type bridge
ip addr add 192.168.200.1/24 dev chvm0
ip link set chvm0 up
echo 1 > /proc/sys/net/ipv4/ip_forward
iptables -t nat -A POSTROUTING -s 192.168.200.0/24 ! -o chvm0 -j MASQUERADE
iptables -A FORWARD -i chvm0 -j ACCEPT
iptables -A FORWARD -o chvm0 -m state --state RELATED,ESTABLISHED -j ACCEPT
```

### Port Forwarding

For each `-p HOST:GUEST` mapping, three iptables rules are created:

1. **PREROUTING DNAT**: External traffic to host port -> guest IP:guest port
2. **OUTPUT DNAT**: Localhost traffic to host port -> guest IP:guest port (for `curl localhost:8080`)
3. **POSTROUTING MASQUERADE**: Hairpin NAT for guest-to-self connections

### Mycelium Backend

**File:** `crates/chvm-lib/src/network/mycelium.rs` (~38 lines)

Extends TAP networking by passing mycelium peer addresses to the guest via kernel cmdline (`chvm.mycelium_peer=`). During storage preparation, when the network mode is `mycelium`, the `mycelium` binary is injected into the guest rootfs at `/usr/bin/mycelium`. The guest init starts the mycelium daemon, which creates a TUN interface with a globally-unique IPv6 address.

After the VM boots, the manager probes the guest (via vsock exec) to discover the mycelium IPv6 address and stores it in `network_state.mycelium_ip`, making it visible in `chvm inspect` output.

Binary search order for mycelium: `./bin/mycelium`, `/usr/local/bin/mycelium`, `/usr/bin/mycelium`, `~/.chvm/bin/mycelium`.

---

## Storage Backends

### StorageBackend Trait

**File:** `crates/chvm-lib/src/storage/traits.rs` (~30 lines)

```rust
#[async_trait]
pub trait StorageBackend: Send + Sync {
    fn name(&self) -> &str;
    async fn prepare(&self, config: &PrepareConfig) -> Result<StorageResult>;
    async fn cleanup(&self, vm_id: &str) -> Result<()>;
}
```

### Block Storage

**Files:** `crates/chvm-lib/src/storage/block.rs` (~34 lines), `image.rs` (~287 lines)

Creates an ext4 disk image:

1. Calculate size: `du -sb rootfs` \* 1.5, minimum 256MB
2. Create sparse file: `truncate -s <size> rootfs.img`
3. Format: `mkfs.ext4 -F -L rootfs rootfs.img`
4. Loop mount: `mount -o loop rootfs.img /tmp/mount`
5. Copy contents: `cp -a rootfs/* /tmp/mount/`
6. **Inject chvm-init**: Copy the statically-linked musl binary to `/chvm-init`
7. **Inject kernel modules**: Copy and decompress vsock + virtiofs `.ko` files to `/lib/modules/`
8. **Inject mycelium** (if network mode is `mycelium`): Copy the `mycelium` binary to `/usr/bin/mycelium`
9. Unmount

Binary search order for chvm-init:

1. `./target/x86_64-unknown-linux-musl/release/chvm-init`
2. `./target/x86_64-unknown-linux-musl/debug/chvm-init`
3. `./target/release/chvm-init`
4. `./target/debug/chvm-init`
5. `~/.chvm/bin/chvm-init`
6. `/usr/local/bin/chvm-init`

Kernel modules injected:

- `vsock.ko` -- Core vsock support
- `vmw_vsock_virtio_transport_common.ko` -- Vsock transport commons
- `vmw_vsock_virtio_transport.ko` -- Virtio vsock transport
- `virtiofs.ko` -- VirtioFS filesystem support

Modules are found from `/lib/modules/<kernel-version>/kernel/` and decompressed from `.ko.zst`, `.ko.xz`, or `.ko.gz` as needed.

### VirtioFS Storage

**File:** `crates/chvm-lib/src/storage/virtiofs.rs` (~120 lines)

Uses an overlay filesystem to provide a writable layer over the read-only cached rootfs:

```
lowerdir = ~/.chvm/images/<digest>/rootfs/  (read-only)
upperdir = ~/.chvm/vms/<id>/storage/upper/  (writes go here)
workdir  = ~/.chvm/vms/<id>/storage/work/
merged   = ~/.chvm/vms/<id>/storage/merged/ (presented to virtiofsd)
```

After mounting the overlay:

1. **Inject chvm-init** into the merged directory (same search paths as block storage)
2. Launch virtiofsd pointing at the merged directory
3. The kernel cmdline is set to `root=rootfs rootfstype=virtiofs` (vs `root=/dev/vda` for block)

Launches virtiofsd:

```
virtiofsd --socket-path=<sock> --shared-dir=<merged> --cache=auto
```

**Kernel requirement:** The guest kernel must have `CONFIG_FUSE_FS=y` and `CONFIG_VIRTIO_FS=y`. The custom kernel (`vmlinux-chvm-6.6.70`) has both built-in. The host-extracted kernel typically lacks these and will kernel panic.

Binary search order: `virtiofsd`, `/usr/libexec/virtiofsd`, `/usr/lib/virtiofsd`, `/usr/local/bin/virtiofsd`, `/usr/bin/virtiofsd`.

---

## Vsock Communication

### Architecture

```
Host                          Cloud Hypervisor              Guest VM
                              (vsock proxy)
chvm exec myvm ls /
  |
  +---> Unix socket ---------> vsock.sock ---------> AF_VSOCK port 1024
        "CONNECT 1024\n"                              vsock_server
        "OK\n"                                          |
        ExecRequest JSON  --------------------------->  exec_handler
        OUTPUT:line1\n    <---------------------------    |
        OUTPUT:line2\n    <---------------------------    |
        ExecResponse JSON <---------------------------  (exit code)
```

Cloud Hypervisor acts as a vsock-to-Unix-socket proxy. The host connects to the Unix socket and sends `CONNECT <port>\n`. CH forwards the connection to the guest's AF_VSOCK listener on that port.

### Protocol Details

**Port 1024 -- Exec**

Request (JSON, one line):

```json
{ "cmd": ["ls", "/"], "env": ["FOO=bar"], "working_dir": "/tmp" }
```

Response (streamed):

```
OUTPUT:bin
OUTPUT:etc
OUTPUT:usr
{"exit_code": 0, "error": ""}
```

Each `OUTPUT:` line is a line of stdout or stderr from the command. The final line is a JSON `ExecResponse` with the exit code.

The host proxy provides two variants: `exec_command` (prints output to stdout) and `exec_capture` (captures output to a string, used internally for mycelium IP probing).

**Port 1025 -- Shell**

No JSON protocol. After connection, the guest opens a PTY, spawns a shell, and proxies raw bytes bidirectionally between the vsock fd and the PTY. The host puts the terminal in raw mode and forwards stdin/stdout.

---

## Guest Init (chvm-init)

**File:** `crates/chvm-init/src/main.rs` (~160 lines)

The init process runs as PID 1 inside the guest. It bootstraps the entire guest environment.

### Boot Sequence

```
1. Set PATH
2. Mount essential filesystems (proc, sys, dev, devpts, tmpfs)
3. Create device nodes (/dev/null, zero, random, urandom, console, tty, net/tun)
4. Mount rootfs (virtiofs mode: mount + pivot_root)
5. Configure network (loopback, eth0, IP, gateway, DNS)
6. Load kernel modules (vsock, virtiofs via init_module syscall)
7. Mount volumes (virtiofs with tags from cmdline)
8. Start mycelium (if configured)
9. Set hostname to "chvm"
10. Start vsock servers (exec port 1024, shell port 1025)
11. Parse and execute user command
12. Reap zombies via SIGCHLD signal handler (sigwait-based, no polling)
13. On process exit: poweroff
```

### Module Loading

Uses the `init_module` syscall directly (no `modprobe` or `insmod` dependency):

```rust
let data = std::fs::read("/lib/modules/vsock.ko")?;
libc::syscall(libc::SYS_init_module, data.as_ptr(), data.len(), "\0".as_ptr());
```

Modules must be loaded in dependency order: vsock -> transport_common -> transport -> virtiofs.

### Configuration Parsing

**File:** `crates/chvm-init/src/cmdline_parser.rs` (~54 lines)

Reads `/proc/cmdline` and extracts `chvm.*` parameters:

| Parameter            | Encoding                | Purpose                    |
| -------------------- | ----------------------- | -------------------------- |
| `chvm.cmd`           | base64 (null-separated) | Command to execute         |
| `chvm.env`           | base64 (null-separated) | Environment variables      |
| `chvm.workdir`       | plain text              | Working directory          |
| `chvm.ip`            | CIDR notation           | Guest IP address           |
| `chvm.gw`            | IP address              | Gateway                    |
| `chvm.mycelium_peer` | URL                     | Mycelium peer (repeatable) |
| `chvm.mounts`        | `tag=path,tag=path`     | Volume mount mappings      |

### Zombie Reaping

The init process reaps child processes using a dedicated thread that blocks on `sigwait(SIGCHLD)`. When a SIGCHLD signal arrives, all available zombie children are reaped in a non-blocking `waitpid` loop. This avoids the overhead of polling.

### Poweroff Sequence

**File:** `crates/chvm-init/src/signal.rs` (~71 lines)

1. `sync()` -- Flush filesystem buffers
2. Attempt to unmount `/proc`, `/sys`, `/dev`
3. Try ACPI poweroff: write `poweroff` to `/sys/power/state`
4. Try sysrq: write `o` to `/proc/sysrq-trigger`
5. Fallback: `reboot(LINUX_REBOOT_CMD_POWER_OFF)` syscall

---

## Kernel Management

**File:** `crates/chvm-lib/src/kernel/manager.rs` (~350 lines)

### Kernel Selection

Resolution order:

1. Explicit `--kernel` flag
2. `config.kernel_path`
3. `~/.chvm/kernels/vmlinux` (default symlink)
4. `~/.chvm/kernels/vmlinux-host` (extracted from host)
5. Auto-extract from running host kernel

For mycelium mode, additionally checks for IPv6 + TUN capabilities.

### Host Kernel Extraction

Extracts the uncompressed vmlinux ELF binary from `/boot/vmlinuz-<version>`:

1. Check if already ELF (some distros ship uncompressed)
2. Try system `extract-vmlinux` script from linux-headers package
3. Scan for gzip magic bytes (`\x1f\x8b`), decompress, check for ELF
4. Scan for zstd (`\x28\xb5\x2f\xfd`), lz4 (`\x02\x21`), xz (`\xfd\x37\x7a\x58\x5a`) magic bytes
5. Validate: extracted file must be >1MB

### Capability Detection

Searches for embedded kernel config (IKCONFIG) or reads `/proc/config.gz`. Checks for key `CONFIG_*` options needed by chvm:

| Config              | Required for                     |
| ------------------- | -------------------------------- |
| `CONFIG_VSOCKETS`   | Vsock exec/attach                |
| `CONFIG_VIRTIO_FS`  | VirtioFS storage + volume mounts |
| `CONFIG_VIRTIO_BLK` | Block device storage             |
| `CONFIG_IPV6`       | Mycelium networking              |
| `CONFIG_TUN`        | Mycelium networking              |

Reports each as `y` (built-in), `m` (module), or `n` (not available).
