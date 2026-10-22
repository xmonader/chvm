# Security Model

This document describes chvm's privilege model, sandboxing posture, and the boundaries of guest isolation. chvm targets single-host, operator-managed deployments: one trusted admin (or small team) runs untrusted *workloads* inside microVMs on a trusted host.

## Privilege model

chvm is invoked per-command (there is no long-running chvm daemon):

- **Mutating operations** — `run`, `create`, `start`, `stop`, `rm`, `exec`, `attach`, `console`, `setup`, `kernel`, `prune`, `doctor --gc` — require **root** (euid 0). They manage `/dev/kvm`, TAP devices, the `chvmbr0` bridge, and iptables DNAT/MASQUERADE rules.
- **Read-only operations** — `list`/`ps`/`ls`, `inspect`, `logs`, `stats`, `images` — are additionally available to members of the **`chvm` group**. `chvm setup` creates the group, grants it read access to the state directory (`/home/<user>/.chvm`, root:`chvm` `0750`), and prints the command to add operators:

  ```bash
  sudo usermod -aG chvm <operator>
  ```

Both gates fail closed with a message explaining how to gain access. Unprivileged users get no socket, state, or log access through chvm itself.

## Process sandboxing

- **cloud-hypervisor** runs with its seccomp filter enabled (`--seccomp true`, the default since long before v52). The VMM's syscall surface is restricted to what it needs; violations kill the offending thread.
- **virtiofsd** runs with its default sandbox (`--sandbox namespace`: user, mount, and pid namespaces; chrooted to the shared directory) and `--seccomp kill`. virtiofsd never sees more of the host filesystem than the directories explicitly mounted into a VM.
- **Guest init** (`chvm-init`) is a statically linked musl binary running as PID 1 inside the guest; it holds no host privileges (the hypervisor grants it only the devices the VM was configured with).

## Guest isolation boundaries

Each VM is a hardware-isolated microVM: dedicated kernel, dedicated memory, virtio devices only. From a VM:

- The host is reachable only through the vsock exec/shell endpoints and the network interfaces chvm configured.
- The host filesystem is visible only via explicitly requested virtiofs mounts (`-v`); there is no host path exposure beyond those directories.
- TAP networking places the VM on a private bridge with NAT; `-p` port forwards and the mycelium overlay are the only inbound paths, both created explicitly per VM.

What chvm does **not** provide (and you should not expect):

- No per-VM CPU pinning or cgroup bandwidth caps beyond `--cpus`/`--memory` sizing (see `chvm stats` for observed usage).
- No VM-level firewall beyond the host iptables rules chvm maintains; guests on the same bridge can talk to each other unless you add rules.
- No multi-tenant scheduling or quota accounting — chvm is one host, one operator model. `hero_compute`/MOS is the control plane built for fleet/multi-tenant use.

## Supply chain

- OCI images are pulled from the configured registries over HTTPS; `chvm login` stores registry credentials in `~/.docker/config.json` (Docker-compatible). The prepared ext4 bases are made immutable (`chmod 0444`) once created, and per-VM disks are qcow2 overlays backed by them.
- Kernel images: `chvm kernel extract-host` extracts the running host kernel; `scripts/build-kernel.sh` builds a minimal known-good kernel from source.

## Reporting

Security issues: please open a private advisory on GitHub rather than a public issue.
