# chvm

A Docker-like CLI for managing lightweight microVMs powered by [Cloud Hypervisor](https://www.cloudhypervisor.org/).

chvm lets you pull OCI container images and run them as hardware-isolated virtual machines with a familiar Docker-style workflow. Each VM boots a real Linux kernel with its own network stack, achieving stronger isolation than containers while maintaining a simple user experience.

## Features

- **Docker-like CLI** -- `chvm run`, `chvm exec`, `chvm attach`, `chvm stop`, `chvm rm`
- **OCI image support** -- Pull images from Docker Hub and other OCI-compatible registries
- **Registry authentication** -- `chvm login`/`chvm logout` with credentials stored in `~/.docker/config.json`
- **Fast boot** -- MicroVMs start in 2-5 seconds
- **TAP networking** -- Bridge-based networking with NAT, port forwarding, and DNS
- **Mycelium networking** -- Peer-to-peer IPv6 overlay network with automatic binary injection and IP tracking
- **Volume mounts** -- Share host directories with VMs via virtiofs
- **Exec & attach** -- Execute commands or get an interactive shell via vsock
- **Kernel management** -- Extract, select, and check host kernels automatically
- **Resource cleanup** -- Automatic garbage collection of leaked TAP devices and iptables rules from crashed VMs
- **Resource control** -- Configure vCPUs, memory, environment variables per VM
- **Short ID resolution** -- Reference VMs by name, full ID, or unique ID prefix (e.g., `chvm stop a1b`)
- **Tab completion** -- Dynamic shell completions for commands, flags, and VM names/IDs

## Requirements

- Linux with KVM support (`/dev/kvm`)
- [Cloud Hypervisor](https://github.com/cloud-hypervisor/cloud-hypervisor) v50+
- [virtiofsd](https://gitlab.com/virtio-fs/virtiofsd) (for volume mounts and virtiofs storage)
- Root privileges (or `doas`/`sudo`) for VM operations
- `mkfs.ext4`, `iptables`, `ip` (iproute2)

## Installation

### From Source

```bash
git clone https://github.com/geomind/chvm.git
cd chvm

# Install musl target (one-time)
rustup target add x86_64-unknown-linux-musl

# Build everything (host CLI + statically-linked guest init)
make build

# Install binaries and shell completions
make install
```

### System Setup

```bash
# Check what's needed
chvm doctor

# Clean up leaked resources from crashed VMs
sudo chvm doctor --gc

# Setup kernel and network bridge
sudo chvm setup
```

## Quick Start

```bash
# Check system readiness
chvm doctor

# Extract a kernel from the running host
sudo chvm kernel extract-host

# Pull an image and run a VM
sudo chvm run --rm alpine:latest echo "Hello from a microVM!"

# Run a detached VM
sudo chvm run -d --name myvm alpine:latest sleep 3600

# Execute a command inside the VM
sudo chvm exec myvm cat /etc/os-release

# Get an interactive shell (detach with Ctrl+P, Ctrl+Q)
sudo chvm attach myvm

# Stop and remove
sudo chvm stop myvm
sudo chvm rm myvm
```

## Usage

### Images

```bash
# Pull from Docker Hub
sudo chvm pull alpine:latest
sudo chvm pull ubuntu:22.04

# Pull from other registries
sudo chvm pull ghcr.io/user/repo:v1

# List cached images
chvm images

# Log in to a registry (credentials stored in ~/.docker/config.json)
chvm login docker.io
chvm login ghcr.io

# Log out
chvm logout docker.io
```

> **Note:** Credentials are stored as base64-encoded tokens in `~/.docker/config.json`

### VM Lifecycle

```bash
# Run foreground (blocks until VM exits)
sudo chvm run --rm alpine:latest echo hello

# Run detached
sudo chvm run -d --name web alpine:latest sleep 3600

# Create without starting
sudo chvm create --name worker alpine:latest sleep 3600
sudo chvm start worker

# Stop, remove
sudo chvm stop web
sudo chvm rm web

# Force remove a running VM
sudo chvm rm -f worker

# Remove all stopped VMs
sudo chvm prune
```

### Inspecting VMs

```bash
# List running VMs
sudo chvm list

# List all VMs (including stopped)
sudo chvm list --all

# Show detailed VM info as JSON
sudo chvm inspect myvm

# Show console logs
sudo chvm logs myvm
```

### Executing Commands

```bash
# Run a command
sudo chvm exec myvm cat /etc/os-release

# Commands with flags work naturally
sudo chvm exec myvm ping -c 4 8.8.8.8

# With environment variables (flags before the container name)
sudo chvm exec -e FOO=bar myvm env

# With working directory
sudo chvm exec -w /tmp myvm pwd

# Interactive shell (detach with Ctrl+P, Ctrl+Q)
sudo chvm attach myvm
```

### Networking

```bash
# Default: TAP networking with bridge (192.168.200.0/24)
sudo chvm run -d --name net-demo alpine:latest sleep 3600

# Port forwarding (tcp by default, or specify /udp)
sudo chvm run -d --name web -p 8080:80 -p 5353:53/udp alpine:latest sleep 3600

# Mycelium peer-to-peer networking
sudo chvm run -d --name myc1 --network mycelium \
  --mycelium-peers tcp://188.40.132.242:9651 \
  --mycelium-peers tcp://136.243.47.186:9651 \
  alpine:latest sleep 3600

# Inspect to see the mycelium IPv6 address
sudo chvm inspect myc1 | jq '.network_state.mycelium_ip'

# No networking
sudo chvm run -d --network none alpine:latest sleep 3600
```

### Resources

```bash
# Custom CPU and memory
sudo chvm run --rm --cpus 2 --memory 1024 alpine:latest free -m

# Environment variables
sudo chvm run --rm -e DB_HOST=localhost -e DB_PORT=5432 alpine:latest env
```

### Storage

```bash
# Mount a host directory into the VM
sudo chvm run --rm -v /path/on/host:/mnt/data alpine:latest ls /mnt/data

# Read-only mount
sudo chvm run --rm -v /path/on/host:/mnt/data:ro alpine:latest cat /mnt/data/file.txt

# Use virtiofs storage backend (requires virtiofsd + kernel with virtiofs support)
sudo chvm run --rm --storage virtiofs alpine:latest echo hello

# Use a local rootfs directory instead of an OCI image
sudo chvm run --rm --rootfs /path/to/rootfs alpine:latest /bin/sh -c "echo custom rootfs"
```

**Storage backends:**

| Backend           | Flag                 | Description                                                                                                                                                                                                         |
| ----------------- | -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `block` (default) | `--storage block`    | Creates an ext4 disk image. Works with any kernel. No extra daemons.                                                                                                                                                |
| `virtiofs`        | `--storage virtiofs` | Uses overlay + virtiofsd. Requires `virtiofsd` and a kernel with `CONFIG_FUSE_FS` + `CONFIG_VIRTIO_FS`. The custom kernel (`vmlinux-chvm-6.6.70`) has these built-in; the host-extracted kernel typically does not. |

See [Kernel Management](#kernel-management) for building a custom kernel.

### Kernel Management

```bash
# List available kernels
chvm kernel list

# Extract kernel from running host
sudo chvm kernel extract-host

# Build a custom minimal kernel (recommended for best performance)
bash scripts/build-kernel.sh
chvm kernel set-default vmlinux-chvm-6.6.70

# Check kernel capabilities
chvm kernel check
```

## Configuration

chvm stores its data in `~/.chvm/` (configurable via `CHVM_HOME`):

```
~/.chvm/
  config.json       # Global configuration
  bin/              # Helper binaries (chvm-init, mycelium)
  kernels/          # Kernel binaries
  images/           # OCI image cache
  vms/              # Per-VM state and data
```

### config.json

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

## Examples

### Web Server Over Mycelium

Run nginx in a VM and access it from anywhere on the mycelium network — no port forwarding needed:

```bash
# Start nginx with mycelium networking
sudo chvm run -d --name web --network mycelium \
  --mycelium-peers tcp://188.40.132.242:9651 \
  --mycelium-peers tcp://136.243.47.186:9651 \
  alpine:latest -- sh -c "\
    apk add --no-cache nginx && mkdir -p /var/www /run/nginx && \
    echo 'hello from chvm' > /var/www/index.html && \
    printf 'server { listen 80; listen [::]:80; root /var/www; }\n' \
      > /etc/nginx/http.d/default.conf && \
    nginx -g 'daemon off;'"

# Get the mycelium IPv6 address
WEB_IP=$(sudo chvm inspect web | jq -r '.network_state.mycelium_ip')

# Access from another VM on the mycelium network
sudo chvm run --rm --network mycelium \
  --mycelium-peers tcp://188.40.132.242:9651 \
  alpine:latest -- sh -c "apk add --no-cache curl && curl -s http://[$WEB_IP]/"
# Output: hello from chvm
```

### Isolated Build Environment

Mount source code into a VM for sandboxed compilation:

```bash
sudo chvm run --rm --cpus 2 --memory 1024 \
  -v /home/user/project:/src:ro \
  -v /tmp/output:/out \
  alpine:latest -- sh -c "\
    apk add --no-cache gcc musl-dev make && \
    cd /src && make && cp build/app /out/"
```

### Disposable Shell

Run untrusted code in a VM that auto-cleans on exit:

```bash
sudo chvm run --rm alpine:latest -- sh -c "wget -qO- https://example.com/script.sh | sh"
```

See [docs/user-guide.md](docs/user-guide.md) for more examples.

## Architecture

chvm is structured as a Rust workspace with three crates:

| Crate       | Purpose                                                               |
| ----------- | --------------------------------------------------------------------- |
| `chvm-cli`  | CLI binary with clap-based command parsing                            |
| `chvm-lib`  | Core library: VM management, OCI, networking, storage, hypervisor API |
| `chvm-init` | Guest init binary (PID 1 inside VMs), statically linked with musl     |

See [docs/architecture.md](docs/architecture.md) for detailed design documentation.

## Documentation

- [Concepts](docs/concepts.md) -- Core concepts, how components interact, end-to-end flows
- [Architecture](docs/architecture.md) -- System design, crate structure, data flow
- [Components](docs/components.md) -- Detailed component design for each subsystem
- [User Guide](docs/user-guide.md) -- Complete CLI reference and configuration
- [Tutorial](docs/tutorial.md) -- Step-by-step getting started guide
- [Testing](docs/testing.md) -- Test suite structure and how to run tests

## License

MIT
