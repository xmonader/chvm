# Concepts

This document explains the core concepts behind chvm and how all the pieces fit together.

## What is chvm?

chvm runs OCI container images inside hardware-isolated virtual machines. Where Docker shares the host kernel with containers, chvm boots a real Linux kernel for each VM. You get a Docker-like CLI (`run`, `exec`, `attach`, `stop`, `rm`) but with full VM isolation.

```
Docker:     Image -> Container (shared kernel, namespaces, cgroups)
chvm:       Image -> MicroVM   (dedicated kernel, hardware isolation via KVM)
```

## Core Concepts

### Images

An **image** is an OCI container image pulled from a registry (Docker Hub, GHCR, etc.). chvm uses the same image format as Docker/Podman. Images are pulled layer-by-layer, extracted into a rootfs directory, and cached locally.

```
chvm pull alpine:latest
  -> downloads OCI manifest + config + layers
  -> extracts tar layers with whiteout handling
  -> caches at ~/.chvm/images/<digest>/rootfs/
```

Currently only **public images** are supported (anonymous auth). Private registries require credentials which are not yet implemented.

### VMs

A **VM** is an instance of an image running inside a Cloud Hypervisor microVM. Each VM has:

- A unique 12-character hex ID (truncated UUIDv4)
- An optional human-readable name
- Its own Linux kernel, network stack, and filesystem
- A vsock channel for host communication (exec/attach)

VMs go through these states:

```
Created  --(start)-->  Running  --(stop/exit/crash)-->  Stopped
                                                            |
                                                        (start) -> Running again

Any state  --(rm -f)--> deleted from disk
```

There is no "Removed" state -- `chvm rm` deletes the VM directory entirely.

### Kernel

chvm needs a Linux kernel binary (`vmlinux`) to boot VMs. Two options:

| Kernel | Size | VirtioFS | Module injection | Boot speed |
|--------|------|----------|-----------------|------------|
| **Custom** (`vmlinux-chvm-6.6.70`) | ~23MB | Built-in | Not needed | Faster |
| **Host-extracted** (`vmlinux-host`) | ~63MB | Usually missing | Required (vsock, virtiofs) | Slower |

The custom kernel has all drivers compiled in (`CONFIG_MODULES=n`), so no module injection is needed. The host kernel requires vsock/virtiofs `.ko` files to be injected into the rootfs image at boot.

### Storage Backends

The **storage backend** determines how the rootfs is presented to the guest kernel.

**Block (default):** Creates a sparse ext4 disk image from the OCI rootfs. Cloud Hypervisor presents it as `/dev/vda`. Works with any kernel.

```
OCI rootfs -> cp to ext4 image -> inject chvm-init + modules -> /dev/vda
Kernel cmdline: root=/dev/vda rw
```

**VirtioFS:** Creates an overlay filesystem over the cached rootfs and serves it via `virtiofsd`. The kernel mounts it directly as the root filesystem. Requires a kernel with `CONFIG_FUSE_FS` and `CONFIG_VIRTIO_FS` (the custom kernel has these; the host kernel typically does not).

```
OCI rootfs (read-only) -> overlay (writable upper) -> inject chvm-init -> virtiofsd -> virtiofs mount
Kernel cmdline: root=rootfs rootfstype=virtiofs rw
```

### Networking

**TAP (default):** Each VM gets a TAP device attached to a Linux bridge (`chvm0`, subnet `192.168.200.0/24`). The host acts as the gateway at `192.168.200.1`. NAT masquerading provides internet access. DNS is configured as `8.8.8.8`, `8.8.4.4`, `1.1.1.1`.

```
Internet <-- NAT --> chvm0 bridge (192.168.200.1) <-- TAP --> VM (192.168.200.x)
```

IPs are dynamically allocated by scanning existing bridge devices to find the first unused address.

**Mycelium:** Extends TAP networking with a peer-to-peer IPv6 overlay. The `mycelium` binary is injected into the guest and started by chvm-init. Each VM gets a globally-unique IPv6 address.

**None:** No network configuration. vsock (exec/attach) still works.

**Port forwarding:** iptables DNAT rules forward host ports to guest ports. Supports TCP (default) and UDP (`-p 8080:80/udp`).

### vsock

**Vsock** (virtio-socket) is the host-guest communication channel. It bypasses the network entirely -- it works even with `--network none`. Cloud Hypervisor acts as a proxy between a Unix socket on the host and `AF_VSOCK` inside the guest.

Two ports are used:
- **Port 1024** -- exec: send a JSON command, get streamed output + exit code
- **Port 1025** -- shell: bidirectional PTY-based interactive session

### chvm-init

**chvm-init** is the guest init process (PID 1). It's a statically-linked musl binary that bootstraps the guest environment:

1. Mount essential filesystems (`/proc`, `/sys`, `/dev`, `/run`)
2. Create device nodes (`/dev/null`, `/dev/zero`, `/dev/random`, etc.)
3. Detect if rootfs needs mounting (virtiofs) or is already mounted (block device)
4. Configure network (static IP from kernel cmdline, or DHCP fallback)
5. Load kernel modules if present (vsock, virtiofs)
6. Mount virtiofs volumes
7. Start mycelium daemon if configured
8. Start vsock servers (exec on port 1024, shell on port 1025)
9. Parse and execute the user's command from kernel cmdline
10. On process exit, trigger ACPI poweroff

### Kernel Cmdline

The kernel command line is the primary config transport from host to guest. Complex data (commands, environment variables) is base64-encoded:

```
console=hvc0 root=/dev/vda rw init=/chvm-init
  chvm.cmd=<base64(echo\0hello)>
  chvm.env=<base64(FOO=bar\0BAZ=qux)>
  chvm.workdir=/app
  chvm.ip=192.168.200.2/24
  chvm.gw=192.168.200.1
  chvm.mycelium_peer=tcp://188.40.132.242:9651
  chvm.mount_0=/mnt/data
```

This avoids needing a secondary config file or network-based configuration.

---

## How Things Fit Together

### The Full `chvm run` Flow

Here's every interaction when you run `chvm run -d --name myvm -e FOO=bar -p 8080:80 alpine:latest sleep 3600`:

```
                    HOST SIDE                                    GUEST SIDE
                    =========                                    ==========

1. CLI (chvm-cli)
   |
   | parse args via clap
   v
2. VmManager.create()
   |
   | check image cache -> pull if missing
   | normalize reference: alpine:latest -> docker.io/library/alpine:latest
   | download OCI manifest, config, layers (streaming to disk)
   | extract layers to ~/.chvm/images/<digest>/rootfs/
   |
   | VmBuilder merges: CLI args + OCI image config + global defaults
   | generate VM ID (UUID), MAC address, vsock CID (FNV-1a hash)
   | build kernel cmdline with base64-encoded cmd/env/workdir
   |
   | write state.json (status: Created)
   v
3. VmManager.start()
   |
   |-- Storage Backend (block or virtiofs)
   |   |
   |   | [block]: create ext4 image, cp rootfs, inject chvm-init + modules
   |   | [virtiofs]: create overlay, inject chvm-init, start virtiofsd
   |   v
   |
   |-- Network Backend (tap, mycelium, or none)
   |   |
   |   | create TAP device, attach to chvm0 bridge
   |   | allocate IP from 192.168.200.0/24
   |   | configure iptables DNAT for port forwards
   |   | configure NAT masquerade
   |   v
   |
   |-- Volume Mounts
   |   |
   |   | for each -v mount: start a virtiofsd instance with unique tag
   |   v
   |
   |-- Launch Cloud Hypervisor
   |   |
   |   | assemble CH args: --kernel, --disk/--fs, --net, --vsock, --cmdline
   |   | spawn cloud-hypervisor process
   |   | wait for API socket (up to 30s)
   |   v
   |
   | update state.json (status: Running, PID)
   |
   |                                              4. Kernel boots
   |                                                 |
   |                                                 v
   |                                              5. /chvm-init (PID 1)
   |                                                 |
   |                                                 | mount /proc, /sys, /dev
   |                                                 | create device nodes
   |                                                 | detect rootfs type
   |                                                 | configure eth0 (IP/gateway/DNS)
   |                                                 | load kernel modules (if needed)
   |                                                 | mount virtiofs volumes
   |                                                 | start mycelium (if configured)
   |                                                 |
   |                                                 | start vsock exec server (port 1024)
   |                                                 | start vsock shell server (port 1025)
   |                                                 |
   |                                                 | parse /proc/cmdline
   |                                                 | decode base64 cmd/env
   |                                                 | exec: sleep 3600
   |                                                 v
   |                                              6. User process runs
   |                                                 |
   |   [mycelium]: probe guest for IPv6 addr  <------|  (vsock exec)
   |   store mycelium_ip in state.json               |
   v                                                 v
   done (detached mode)                           VM running...
```

### The `chvm exec` Flow

```
Host                          Cloud Hypervisor              Guest VM
----                          ----------------              --------

chvm exec myvm ls /
  |
  v
connect to ~/.chvm/vms/<id>/vsock.sock (Unix socket)
  |
  | send "CONNECT 1024\n"
  | recv "OK\n"
  |                           proxy: Unix socket <--> AF_VSOCK
  |
  | send JSON: {"cmd":["ls","/"],"env":[],"working_dir":null}
  |                                                     |
  |                                                     v
  |                                                exec_handler
  |                                                     |
  |                                                  fork+exec: ls /
  |                                                     |
  | recv "OUTPUT:bin\n"            <------- stdout -----|
  | recv "OUTPUT:dev\n"            <------- stdout -----|
  | recv "OUTPUT:etc\n"            <------- stdout -----|
  | ...                                                 |
  | recv JSON: {"exit_code":0}     <---- process exits -|
  |
  v
print output, exit with same code
```

### The `chvm attach` Flow

```
Host                          Cloud Hypervisor              Guest VM
----                          ----------------              --------

chvm attach myvm
  |
  | set terminal to raw mode (cfmakeraw)
  |
  | connect to vsock.sock
  | send "CONNECT 1025\n"
  | recv "OK\n"
  |                           proxy: Unix socket <--> AF_VSOCK
  |                                                     |
  |                                                     v
  |                                                shell_handler
  |                                                     |
  |                                                  allocate PTY
  |                                                  spawn /bin/ash
  |                                                     |
  | stdin -----> vsock -----> PTY master -----> shell   |
  | stdout <---- vsock <----- PTY master <----- shell   |
  |                                                     |
  | (bidirectional byte stream)                         |
  |                                                     |
  | user types "exit"                                   |
  |                                                     v
  | restore terminal mode (on drop)               shell exits
  v
done
```

### The `chvm stop` + `chvm rm` Flow

```
chvm stop myvm
  |
  v
send shutdown via CH REST API (Unix socket)
  |
  | wait for process exit (up to 30s, then SIGKILL)
  |
  | teardown network:
  |   delete TAP device
  |   remove iptables DNAT rules
  |
  | cleanup storage:
  |   kill virtiofsd processes
  |   unmount overlays
  |
  | update state.json (status: Stopped)
  v
done

chvm rm myvm
  |
  v
teardown network (idempotent, safe if already done)
  |
  | cleanup storage
  | delete ~/.chvm/vms/<id>/ directory
  | remove from in-memory state map
  v
done
```

### Component Interaction Map

```
+-------------------+
|    chvm-cli       |  User-facing. Parses args, formats output.
+--------+----------+
         |
         | calls library functions
         v
+-------------------+
|    VmManager      |  Central orchestrator. Owns StateStore + ImageCache.
+--------+----------+
         |
    +----+----+----+----+----+
    |    |    |    |    |    |
    v    v    v    v    v    v

 State  OCI   Network  Storage  Hypervisor  Vsock
 Store  Pull  Backend  Backend  Process     Proxy
  |      |      |        |        |           |
  |      |      |        |        |           |
  v      v      v        v        v           v

state   Docker  TAP     Block    Cloud       Unix
.json   Hub     device  image    Hypervisor  socket
        GHCR    Bridge  VirtioFS process     to
        ...     iptables overlay  REST API   AF_VSOCK
                                              |
                                              v
                                        +-----------+
                                        | chvm-init |  Guest PID 1
                                        +-----+-----+
                                              |
                                    +---------+---------+
                                    |         |         |
                                    v         v         v
                                  exec      shell     user
                                  server    server    process
                                 (1024)    (1025)
```

### State Persistence

All VM state is on disk as JSON + files:

```
~/.chvm/
  config.json                    # Global config (defaults, paths)
  kernels/
    vmlinux -> vmlinux-chvm-6.6.70   # Default kernel (symlink)
    vmlinux-chvm-6.6.70              # Custom kernel (all drivers built-in)
    vmlinux-host                     # Host-extracted kernel
  images/
    index.json                   # Image cache index (ref -> digest mapping)
    <digest>/
      rootfs/                    # Extracted OCI layers (the filesystem tree)
      layer-0-<hash>.tar.gz     # Raw downloaded layers
      layer-1-<hash>.tar.gz
  bin/
    chvm-init                    # Guest init binary (musl static)
    mycelium                     # Mycelium binary (optional)
  vms/
    <vm-id>/
      state.json                 # RuntimeState (config, status, timestamps)
      rootfs.img                 # ext4 block device (block backend)
      storage/                   # virtiofs overlay + socket (virtiofs backend)
        overlay/
          upper/                 # Writable layer
          work/
          merged/                # Union of lower + upper (served by virtiofsd)
        virtiofsd.sock
      api.sock                   # Cloud Hypervisor REST API socket
      vsock.sock                 # Cloud Hypervisor vsock proxy socket
      console.log                # Guest serial console output
      cloud-hypervisor.log       # CH process log
      mount-N/                   # Volume mount virtiofsd instances
        storage/
          overlay/
          virtiofsd.sock
```

### Config Resolution Order

When creating a VM, configuration is merged from multiple sources (later overrides earlier):

```
1. Global defaults (hardcoded: 1 vCPU, 512MB, tap, block)
2. config.json (~/.chvm/config.json)
3. OCI image config (entrypoint, cmd, env, workdir)
4. CLI arguments (--cpus, --memory, -e, -v, etc.)
```

For the command to execute:
```
1. CLI cmd args       -> used if provided
2. Image entrypoint + cmd -> used if no CLI cmd
3. /bin/sh            -> fallback if nothing else
```

For environment variables:
```
1. Image env          -> base set
2. CLI -e flags       -> appended (can override image env)
3. Standard env       -> PATH, HOME, TERM (set by chvm-init)
```
