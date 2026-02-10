# User Guide

## CLI Reference

### Global Options

```
chvm [--verbose] <command>
```

| Option | Description |
|--------|-------------|
| `--verbose` | Enable verbose output (debug logging) |
| `-h, --help` | Print help |
| `-V, --version` | Print version |

---

### chvm pull

Pull an OCI image from a container registry.

```
chvm pull <IMAGE>
```

Images are pulled from Docker Hub by default. Other OCI-compatible registries work too, but only **public images** are supported (anonymous access only — no credential/login support yet).

```bash
chvm pull alpine:latest                        # Docker Hub library image
chvm pull busybox:1.36                         # Specific tag
chvm pull ubuntu:22.04                         # Ubuntu
chvm pull ghcr.io/user/public-repo:v1          # GitHub Container Registry (public only)
```

Registry selection is automatic based on the image reference format:
- `alpine:latest` -- defaults to `docker.io/library/alpine:latest`
- `myuser/myimage` -- defaults to `docker.io/myuser/myimage:latest`
- `ghcr.io/user/repo:v1` -- uses GitHub Container Registry as-is
- References containing a `.` in the hostname part are treated as custom registries

**Note:** Private registries and images requiring authentication are not yet supported. Only publicly accessible images can be pulled.

Images are cached in `~/.chvm/images/`. Re-pulling a cached image is a no-op.

---

### chvm run

Create and start a VM in one step.

```
chvm run [OPTIONS] <IMAGE> [CMD...]
```

| Option | Default | Description |
|--------|---------|-------------|
| `--name <NAME>` | auto-generated | VM name |
| `--cpus <N>` | 1 | Number of vCPUs |
| `--memory <N>` | 512 | Memory in MB |
| `--storage <TYPE>` | block | Storage backend: `block` or `virtiofs` |
| `--network <MODE>` | tap | Network mode: `tap`, `mycelium`, or `none` |
| `-p, --publish <HOST:GUEST[/PROTO]>` | | Port forward, optional `/tcp` or `/udp` (repeatable) |
| `-e, --env <KEY=VALUE>` | | Environment variable (repeatable) |
| `-v, --volume <HOST:GUEST[:ro]>` | | Volume mount (repeatable) |
| `--kernel <PATH>` | | Custom kernel path |
| `-d, --detach` | false | Run in background |
| `--rm` | false | Remove VM when it exits |
| `--rootfs <DIR>` | | Use local rootfs directory instead of image |
| `--mycelium-peers <ADDR>` | | Mycelium peer addresses (repeatable) |

```bash
# Foreground (blocks until exit)
chvm run --rm alpine:latest echo "hello world"

# Detached
chvm run -d --name myvm alpine:latest sleep 3600

# Full example
chvm run -d \
  --name web \
  --cpus 2 \
  --memory 1024 \
  -e APP_ENV=production \
  -p 8080:80 \
  -v /data/app:/app \
  alpine:latest /app/server
```

---

### chvm create

Create a VM without starting it.

```
chvm create [OPTIONS] <IMAGE> [CMD...]
```

Takes the same options as `chvm run` except `-d` and `--rm`. The VM is created in `Created` state and can be started later with `chvm start`.

```bash
chvm create --name worker alpine:latest /usr/bin/worker
chvm start worker
```

---

### chvm start

Start a created or stopped VM.

```
chvm start <CONTAINER>
```

`<CONTAINER>` can be the VM name or ID.

---

### chvm stop

Stop a running VM.

```
chvm stop <CONTAINER>
```

Sends a graceful shutdown request via the Cloud Hypervisor API. If the VM doesn't stop within 30 seconds, it is force-killed.

---

### chvm rm

Remove a VM.

```
chvm rm [-f] <CONTAINER>
```

| Option | Description |
|--------|-------------|
| `-f, --force` | Remove even if running (stops first) |

```bash
chvm rm myvm           # Remove stopped VM
chvm rm -f myvm        # Force remove running VM
```

---

### chvm exec

Execute a command inside a running VM.

```
chvm exec [OPTIONS] <CONTAINER> <CMD...>
```

| Option | Description |
|--------|-------------|
| `-e, --env <KEY=VALUE>` | Set environment variable (repeatable) |
| `-w, --workdir <DIR>` | Set working directory |

```bash
chvm exec myvm echo hello
chvm exec myvm ls -la /
chvm exec myvm ping -c 4 8.8.8.8
chvm exec -e DB=prod myvm env
chvm exec -w /tmp myvm pwd
```

Options like `-e` and `-w` must come before the container name. Everything after the container name is the command. A `--` separator before the command is optional.

---

### chvm attach

Attach an interactive shell to a running VM.

```
chvm attach <CONTAINER>
```

Opens a PTY-based shell session. The shell is determined by what's available in the guest (`bash`, `ash`, or `sh`). Type `exit` to detach.

```bash
chvm attach myvm
# You're now inside the VM
# Type 'exit' to return
```

---

### chvm list

List VMs.

```
chvm list [-a]
```

Aliases: `chvm ps`, `chvm ls`

| Option | Description |
|--------|-------------|
| `-a, --all` | Show all VMs (default: running only) |

```bash
chvm list              # Running VMs only
chvm list --all        # All VMs including stopped
chvm ps                # Alias
chvm ls -a             # Alias with --all
```

---

### chvm logs

Show VM console output.

```
chvm logs [-f] <CONTAINER>
```

| Option | Description |
|--------|-------------|
| `-f, --follow` | Follow log output |

Shows the serial console log from the guest kernel and init process.

---

### chvm inspect

Show detailed VM information as JSON.

```
chvm inspect <CONTAINER>
```

Returns the full `RuntimeState` including VM config, network state, storage state, timestamps, and process IDs. For VMs using mycelium networking, the `network_state.mycelium_ip` field contains the VM's globally-unique IPv6 address.

```bash
# Show full state
chvm inspect myvm

# Extract just the mycelium IP
chvm inspect myvm | jq '.network_state.mycelium_ip'
```

---

### chvm images

List cached OCI images.

```
chvm images
```

Shows reference, digest, size, and pull date for all cached images.

---

### chvm prune

Remove all stopped VMs.

```
chvm prune
```

---

### chvm doctor

Check system requirements.

```
chvm doctor
```

Checks for:
- Cloud Hypervisor binary
- virtiofsd binary
- Kernel availability
- mkfs.ext4
- iptables
- ip (iproute2)
- /dev/kvm access
- Data directory

---

### chvm setup

Setup system dependencies.

```
chvm setup [COMPONENT]
```

Components: `all` (default), `kernel`, `network`

- `kernel`: Extracts the host kernel and sets it as default
- `network`: Creates the chvm0 bridge and enables NAT

---

### chvm kernel

Manage kernels.

```
chvm kernel <SUBCOMMAND>
```

| Subcommand | Description |
|------------|-------------|
| `list` | List available kernels in `~/.chvm/kernels/` |
| `set-default <NAME>` | Set the default kernel (creates vmlinux symlink) |
| `extract-host` | Extract vmlinux from the running host kernel |
| `check [PATH]` | Check kernel capabilities (vsock, virtiofs, IPv6, TUN) |

```bash
chvm kernel list
chvm kernel extract-host
chvm kernel set-default vmlinux-host
chvm kernel check
```

---

### chvm version

Show version information.

```
chvm version
```

Displays chvm version and the detected Cloud Hypervisor version.

---

## Networking

### TAP Networking (Default)

chvm creates a Linux bridge (`chvm0`) with subnet `192.168.200.0/24`. Each VM gets a TAP device attached to this bridge and is dynamically assigned the first unused IP in the range `192.168.200.2-254` (scanning existing bridge devices to avoid collisions).

```
Host (192.168.200.1)
  |
  +-- chvm0 bridge
       |
       +-- tap-<vm1> --- VM1 (192.168.200.2)
       |
       +-- tap-<vm2> --- VM2 (192.168.200.3)
```

NAT masquerading is configured so VMs can access the internet through the host's default route. DNS is configured as 8.8.8.8 / 8.8.4.4 / 1.1.1.1 inside the guest.

If no static IP is passed via the kernel cmdline, the guest init falls back to DHCP, trying `dhclient` and `udhcpc` in order.

### Port Forwarding

Use `-p HOST:GUEST` to forward host ports to guest ports. An optional `/tcp` or `/udp` suffix specifies the protocol (default: tcp):

```bash
chvm run -d --name web -p 8080:80 -p 8443:443 alpine:latest sleep 3600

# UDP port forwarding
chvm run -d --name dns -p 5353:53/udp alpine:latest sleep 3600
```

This creates iptables DNAT rules that forward traffic from the host port to the VM's assigned IP and guest port. Both external and localhost access are supported. Rules are automatically cleaned up when the VM is stopped or removed.

### Mycelium Networking

[Mycelium](https://github.com/threefoldtech/mycelium) provides a peer-to-peer IPv6 overlay network. Each VM gets a globally-unique IPv6 address derived from a generated key pair.

#### Setup

Download the mycelium binary (one-time step):

```bash
mkdir -p ~/.chvm/bin
curl -sL https://github.com/threefoldtech/mycelium/releases/download/v0.7.3/mycelium-x86_64-unknown-linux-musl.tar.gz \
  | tar xz -C ~/.chvm/bin/
chmod +x ~/.chvm/bin/mycelium
```

The mycelium binary is automatically injected into the guest rootfs when `--network mycelium` is used. It is searched in: `./bin/mycelium`, `/usr/local/bin/mycelium`, `/usr/bin/mycelium`, and `~/.chvm/bin/mycelium`.

#### Running

```bash
chvm run -d --name myc1 --network mycelium \
  --mycelium-peers tcp://188.40.132.242:9651 \
  --mycelium-peers tcp://136.243.47.186:9651 \
  alpine:latest sleep 3600
```

After boot, the guest will have both a TAP interface (`eth0`) for local/internet access and a mycelium TUN interface with a global IPv6 address.

#### Inspecting the Mycelium Address

The VM's mycelium IPv6 address is automatically probed after boot and stored in the VM state:

```bash
chvm inspect myc1 | jq '.network_state.mycelium_ip'
# "5d7:bb4e:d6e5:1e5a:2056:279a:5190:1c62"
```

You can also query it from inside the guest:

```bash
chvm exec myc1 ip -6 addr show dev mycelium scope global
chvm exec myc1 mycelium inspect --json --key-file /etc/mycelium/priv_key.bin
```

#### VM-to-VM Communication

Two VMs on the mycelium network can reach each other via their IPv6 addresses:

```bash
# Start two VMs
chvm run -d --name myc1 --network mycelium \
  --mycelium-peers tcp://188.40.132.242:9651 \
  --mycelium-peers tcp://136.243.47.186:9651 \
  alpine:latest sleep 3600
chvm run -d --name myc2 --network mycelium \
  --mycelium-peers tcp://188.40.132.242:9651 \
  --mycelium-peers tcp://136.243.47.186:9651 \
  alpine:latest sleep 3600

# Get myc2's address
MYC2_IP=$(chvm inspect myc2 | jq -r '.network_state.mycelium_ip')

# Ping from myc1 to myc2
chvm exec myc1 ping -6 -c 4 "$MYC2_IP"
```

### No Networking

```bash
chvm run -d --network none alpine:latest sleep 3600
```

Disables all network configuration. The VM has no external connectivity but vsock (exec/attach) still works.

---

## Storage

### Block Device (Default)

The default `block` backend creates a sparse ext4 image from the OCI rootfs:

1. Calculates required size (1.5x rootfs + overhead, minimum 256MB)
2. Creates sparse file and formats with mkfs.ext4
3. Copies rootfs contents, injects chvm-init and kernel modules
4. Cloud Hypervisor presents it as a virtio-block device (`/dev/vda`)

### VirtioFS Backend

The `virtiofs` backend uses virtiofsd to share the rootfs:

```bash
chvm run -d --storage virtiofs --name myvm alpine:latest sleep 3600
```

Creates an overlay filesystem (to allow writes without modifying the cached rootfs), injects `chvm-init` into the overlay, and launches virtiofsd. The kernel boots with `root=rootfs rootfstype=virtiofs` and mounts the virtiofs share directly as the root filesystem.

**Requirements:**
- `virtiofsd` installed (typically at `/usr/libexec/virtiofsd` or `/usr/lib/qemu/virtiofsd` — check with `chvm doctor`)
- A kernel with `CONFIG_FUSE_FS=y` and `CONFIG_VIRTIO_FS=y`

The custom kernel built by `scripts/build-kernel.sh` (`vmlinux-chvm-6.6.70`) has both drivers compiled in and works out of the box. The host-extracted kernel (`vmlinux-host`) typically lacks virtiofs support and will kernel panic when used with `--storage virtiofs`. If you need virtiofs, use the custom kernel:

```bash
# Build and set the custom kernel as default
bash scripts/build-kernel.sh
chvm kernel set-default vmlinux-chvm-6.6.70

# Now virtiofs works
chvm run --rm --storage virtiofs alpine:latest echo hello
```

### Volume Mounts

Share host directories with the guest:

```bash
chvm run -d -v /host/path:/guest/path alpine:latest sleep 3600
chvm run -d -v /data:/mnt/data:ro alpine:latest sleep 3600    # read-only
```

Each volume mount runs its own virtiofsd instance. The guest init mounts them at the specified paths using virtiofs.

---

## Configuration

### Data Directory

Default: `~/.chvm/`

Override with `CHVM_HOME` environment variable. When running under `sudo`/`doas`, chvm resolves the original user's home directory via `SUDO_USER` or `DOAS_USER`.

### config.json

Location: `~/.chvm/config.json`

```json
{
  "base_dir": "/home/user/.chvm",
  "default_vcpu": 1,
  "default_memory_mb": 512,
  "default_network_mode": "tap",
  "default_storage_backend": "block",
  "default_bridge": "chvm0",
  "kernel_path": null,
  "virtiofsd_bin": null,
  "cloud_hypervisor_bin": null
}
```

All fields are optional and fall back to defaults if not specified. Set `kernel_path` to override the default kernel, `virtiofsd_bin` to specify a custom virtiofsd path, or `cloud_hypervisor_bin` to override the CH binary location.

### Environment Variables

| Variable | Description |
|----------|-------------|
| `CHVM_HOME` | Override data directory (default: `~/.chvm`) |
| `CHVM_CH_BIN` | Override Cloud Hypervisor binary path |

---

## Building a Custom Kernel

For best performance, build a minimal kernel with all drivers built-in (no modules):

```bash
bash scripts/build-kernel.sh
```

This builds a ~23MB kernel with vsock, virtiofs, TUN, IPv6, and PVH support all built-in. Compared to using the host kernel (~64MB with separate modules), a custom kernel:

- Is 3x smaller (23MB vs 64MB)
- Boots faster
- Eliminates module injection into the rootfs image
- Eliminates the `load_kernel_modules()` step in the guest init

The script downloads the kernel source from kernel.org, applies a minimal config, and builds a stripped `vmlinux`. Output: `~/.chvm/kernels/vmlinux-chvm-6.6.70`.

```bash
# Build with defaults (kernel 6.6.70)
bash scripts/build-kernel.sh

# Build a specific version
bash scripts/build-kernel.sh --version 6.12.5

# Set as default
chvm kernel set-default vmlinux-chvm-6.6.70
```

### Build Requirements

- `gcc`, `make`, `flex`, `bison`, `bc`
- `libelf-dev`, `libssl-dev`

On Ubuntu/Debian: `sudo apt install gcc make flex bison bc libelf-dev libssl-dev`

---

## Troubleshooting

### "Permission denied" or KVM errors

chvm requires root privileges for VM operations. Use `sudo` or `doas`:

```bash
sudo chvm run --rm alpine:latest echo hello
```

Ensure `/dev/kvm` is accessible:

```bash
ls -la /dev/kvm
# Should show: crw-rw---- root kvm /dev/kvm
```

### "Kernel not found"

Extract a kernel from your running host:

```bash
sudo chvm kernel extract-host
chvm kernel list
```

### "virtiofsd not found"

Install virtiofsd. On Ubuntu/Debian:

```bash
sudo apt install virtiofsd
```

Or build from source: https://gitlab.com/virtio-fs/virtiofsd

### VM won't start

Run `chvm doctor` to check all dependencies:

```bash
chvm doctor
```

Check Cloud Hypervisor logs:

```bash
cat ~/.chvm/vms/<vm-id>/cloud-hypervisor.log
```

### exec/attach times out

The guest needs a few seconds to boot and start the vsock servers. Wait 3-5 seconds after `chvm run -d` before using `chvm exec` or `chvm attach`.

### Network connectivity issues

Verify the bridge exists and NAT is configured:

```bash
ip link show chvm0
iptables -t nat -L POSTROUTING -n
```

Re-setup networking:

```bash
sudo chvm setup network
```

---

## Known Limitations

### Kernel

**Kernel module injection uses the host kernel version.** When creating a block device image, chvm injects vsock/virtiofs modules from `/lib/modules/$(uname -r)/`. If you boot the VM with a different kernel version (e.g., a custom-built kernel), these modules will fail to load due to version mismatch. This does not affect custom kernels built with `CONFIG_MODULES=n` (all drivers built-in), since module injection is automatically skipped.

**`chvm kernel check` requires embedded config.** Capability detection (`chvm kernel check`) only works on kernels built with `CONFIG_IKCONFIG=y`. For kernels without embedded config, the check will report "could not detect kernel capabilities" even if the kernel is functional.

### Custom Kernel Build

**x86_64 only.** The build script and kernel config target x86_64 / amd64 only. ARM64 / aarch64 is not currently supported.

**Build requires ~5GB disk space.** The kernel source download (~120MB), extraction (~900MB), and build artifacts (~3GB) need approximately 5GB of free space in the build directory (`/tmp/chvm-kernel-build` by default). The script warns if disk space is low but does not abort.

**Dependency hints are best-effort.** The build script detects your package manager (apt, dnf, pacman, apk, zypper) and suggests appropriate install commands. If your system uses a different package manager, you'll need to install the equivalent packages manually: `gcc`, `make`, `flex`, `bison`, `bc`, `libelf-dev`, `libssl-dev`.

**GCC 15+ compatibility patch is fragile.** Kernel versions older than ~6.8 need a patch for GCC 15's C23 `bool`/`true`/`false` keywords. The script applies this automatically, but the patch uses pattern matching on kernel source files and may break with unusual kernel versions or significant source changes.

### VirtioFS Storage

**VirtioFS requires kernel virtiofs support.** The `--storage virtiofs` backend needs a kernel with `CONFIG_FUSE_FS=y` and `CONFIG_VIRTIO_FS=y`. The host-extracted kernel (`vmlinux-host`) typically does not have these, causing a kernel panic (`VFS: Unable to mount root fs`). Use the custom kernel (`vmlinux-chvm-6.6.70`) built by `scripts/build-kernel.sh`, which has both drivers compiled in.

**VirtioFS requires virtiofsd.** The `virtiofsd` binary is needed but is often installed to non-standard paths like `/usr/libexec/virtiofsd` or `/usr/lib/qemu/virtiofsd`. chvm searches these paths automatically. On Ubuntu/Debian: `sudo apt install virtiofsd`.

### Storage and Module Injection

**Module decompression is best-effort.** When injecting kernel modules, compressed `.ko.zst`, `.ko.xz`, and `.ko.gz` variants are decompressed using host tools (`zstd`, `xz`, `gzip`). If these tools are missing, the compressed modules are silently skipped. Run `chvm doctor` to check for missing tools.

**No `depmod` is run.** Injected modules are copied flat into `/lib/modules/` without running `depmod` to build dependency metadata. The guest init loads modules in a hardcoded dependency order, which works for the specific vsock/virtiofs modules but may not generalize to other module sets.
