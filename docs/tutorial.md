# Tutorial: Getting Started with chvm

This tutorial walks you through setting up chvm and running your first microVMs.

## Prerequisites

You need a Linux system with:

- KVM support (check with `ls /dev/kvm`)
- Rust toolchain (for building from source)
- Root access (via `sudo` or `doas`)

## Step 1: Build chvm

Clone and build the project:

```bash
git clone https://github.com/xmonader/chvm.git
cd chvm

# Install musl target (one-time)
rustup target add x86_64-unknown-linux-musl

# Build everything (host CLI + statically-linked guest init)
make build
```

Install the binaries and shell completions:

```bash
make install
```

This places `chvm` in `~/.cargo/bin/`, `chvm-init` in `~/.chvm/bin/`, and installs bash tab completions. After installation, restart your shell or run:

```bash
source ~/.local/share/bash-completion/completions/chvm
```

You can now tab-complete commands, flags, and VM names/IDs.

## Step 2: Check System Requirements

Run the doctor command to verify everything is in place:

```bash
chvm doctor
```

You should see output like:

```
CHVM Doctor - System Check

Cloud Hypervisor: OK (cloud-hypervisor) - cloud-hypervisor v50.0.0
virtiofsd: OK
Kernel: OK (/home/user/.chvm/kernels/vmlinux) - 63MB
mkfs.ext4: OK
iptables: OK
ip command: OK
/dev/kvm: OK
Data directory: OK (/home/user/.chvm)

All checks passed! CHVM is ready to use.
```

If Cloud Hypervisor is missing, install it:

```bash
# Download the latest release
wget https://github.com/cloud-hypervisor/cloud-hypervisor/releases/latest/download/cloud-hypervisor
chmod +x cloud-hypervisor
sudo mv cloud-hypervisor /usr/local/bin/
```

## Step 3: Setup Kernel

chvm needs a Linux kernel binary to boot VMs. The easiest way is to extract one from your running host:

```bash
sudo chvm kernel extract-host
```

Verify it's available:

```bash
chvm kernel list
```

You should see `vmlinux-host` listed. Check its capabilities:

```bash
chvm kernel check
```

## Step 4: Pull an Image

Pull an Alpine Linux image from Docker Hub:

```bash
sudo chvm pull alpine:latest
```

This downloads the OCI image layers and caches them locally. Verify:

```bash
chvm images
```

## Step 5: Run Your First VM

Run a simple foreground command:

```bash
sudo chvm run --rm alpine:latest echo "Hello from a microVM!"
```

The `--rm` flag removes the VM after it exits. You should see:

```
Hello from a microVM!
```

That command:
1. Created a VM from the Alpine image
2. Booted a Linux kernel
3. Started the chvm-init process inside the guest
4. Ran `echo "Hello from a microVM!"`
5. Shut down the VM and cleaned up

## Step 6: Run a Detached VM

Run a long-lived VM in the background:

```bash
sudo chvm run -d --name tutorial-vm alpine:latest sleep 3600
```

Check it's running:

```bash
sudo chvm list
```

You should see your VM with status `running`.

## Step 7: Execute Commands

Run commands inside the running VM:

```bash
# Check the OS
sudo chvm exec tutorial-vm cat /etc/os-release

# List files
sudo chvm exec tutorial-vm ls /

# Check memory
sudo chvm exec tutorial-vm free -m

# Check network connectivity
sudo chvm exec tutorial-vm ping -c 4 8.8.8.8

# Set environment variables
sudo chvm exec -e GREETING=hello tutorial-vm env

# Work in a specific directory
sudo chvm exec -w /tmp tutorial-vm pwd
```

## Step 8: Attach an Interactive Shell

Get a shell inside the VM:

```bash
sudo chvm attach tutorial-vm
```

You're now inside the VM. Try some commands:

```
/ # uname -a
Linux chvm 6.17.0-12-generic #1 SMP ...
/ # hostname
chvm
/ # ip addr show eth0
...
/ # exit
```

Type `exit` to return to the host.

## Step 9: Inspect the VM

View detailed VM information:

```bash
sudo chvm inspect tutorial-vm
```

This outputs the full VM state as JSON, including configuration, network state, storage state, and timestamps.

View console logs:

```bash
sudo chvm logs tutorial-vm
```

## Step 10: Stop and Clean Up

Stop the VM:

```bash
sudo chvm stop tutorial-vm
```

Verify it stopped:

```bash
sudo chvm list --all
```

The VM should show status `stopped`. Remove it:

```bash
sudo chvm rm tutorial-vm
```

## Next Steps: Advanced Features

### Port Forwarding

Run a VM with port forwards:

```bash
sudo chvm run -d --name web -p 8080:80 alpine:latest sleep 3600
```

Port 8080 on the host now forwards to port 80 in the VM. If you start a web server inside the VM, it's accessible at `http://localhost:8080`.

### Volume Mounts

Share files between host and guest:

```bash
# Create a test directory
mkdir -p /tmp/shared
echo "hello from host" > /tmp/shared/greeting.txt

# Run with a volume mount
sudo chvm run --rm -v /tmp/shared:/mnt/data alpine:latest cat /mnt/data/greeting.txt
```

Output: `hello from host`

### Custom Resources

Configure CPU and memory:

```bash
sudo chvm run --rm --cpus 2 --memory 1024 alpine:latest free -m
```

### Environment Variables

Pass configuration to the guest:

```bash
sudo chvm run --rm \
  -e DATABASE_URL=postgres://db:5432/app \
  -e APP_ENV=production \
  alpine:latest env
```

### Multiple VMs

Run multiple VMs simultaneously:

```bash
sudo chvm run -d --name vm1 alpine:latest sleep 3600
sudo chvm run -d --name vm2 alpine:latest sleep 3600
sudo chvm run -d --name vm3 alpine:latest sleep 3600

sudo chvm list

# Each VM has its own isolated network, filesystem, and resources
sudo chvm exec vm1 hostname
sudo chvm exec vm2 hostname

# Clean up all at once
sudo chvm stop vm1 && sudo chvm stop vm2 && sudo chvm stop vm3
sudo chvm prune
```

### Using a Local Rootfs

You can use a local directory as the rootfs instead of pulling an OCI image:

```bash
# Use a debootstrap rootfs, a custom Alpine rootfs, etc.
sudo chvm run --rm --rootfs /path/to/my/rootfs -- /bin/sh -c "echo custom rootfs"
```

The directory must contain standard Linux filesystem structure (`/bin`, `/usr`, `/lib`, `/etc`).
