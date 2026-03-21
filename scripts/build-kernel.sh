#!/bin/bash
#
# build-kernel.sh - Build a minimal Linux kernel for chvm microVMs
#
# Builds a stripped vmlinux with all needed drivers built-in (no modules).
# Adapted from UVM's build-kernel.sh for Cloud Hypervisor + chvm.
#
# Usage:
#   bash scripts/build-kernel.sh [--version 6.6.70] [--output-dir ~/.chvm/kernels]
#
# Requirements:
#   gcc, make, flex, bison, bc, libelf-dev, libssl-dev
#

set -euo pipefail

# Defaults
KERNEL_VERSION="${KERNEL_VERSION:-6.6.70}"
KERNEL_MAJOR="${KERNEL_VERSION%%.*}"
OUTPUT_DIR="${OUTPUT_DIR:-${HOME}/.chvm/kernels}"
BUILD_DIR="/tmp/chvm-kernel-build"
JOBS="$(nproc)"
OUTPUT_NAME=""

# Parse arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --version)
            KERNEL_VERSION="$2"
            KERNEL_MAJOR="${KERNEL_VERSION%%.*}"
            shift 2
            ;;
        --output-dir)
            OUTPUT_DIR="$2"
            shift 2
            ;;
        --build-dir)
            BUILD_DIR="$2"
            shift 2
            ;;
        --jobs|-j)
            JOBS="$2"
            shift 2
            ;;
        --help|-h)
            echo "Usage: $0 [OPTIONS]"
            echo ""
            echo "Build a minimal Linux kernel for chvm microVMs."
            echo ""
            echo "Options:"
            echo "  --version VERSION   Kernel version to build (default: 6.6.70)"
            echo "  --output-dir DIR    Output directory (default: ~/.chvm/kernels)"
            echo "  --build-dir DIR     Build directory (default: /tmp/chvm-kernel-build)"
            echo "  --jobs N            Number of parallel build jobs (default: nproc)"
            echo "  -h, --help          Show this help"
            exit 0
            ;;
        *)
            echo "Unknown option: $1"
            exit 1
            ;;
    esac
done

OUTPUT_NAME="vmlinux-chvm-${KERNEL_VERSION}"

echo "=== chvm kernel builder ==="
echo "  Kernel version: ${KERNEL_VERSION}"
echo "  Output: ${OUTPUT_DIR}/${OUTPUT_NAME}"
echo "  Build dir: ${BUILD_DIR}"
echo "  Jobs: ${JOBS}"
echo ""

# ---- Check build dependencies ----

# Detect package manager for install hints
detect_install_hint() {
    if command -v apt &>/dev/null; then
        echo "sudo apt install gcc make flex bison bc libelf-dev libssl-dev"
    elif command -v dnf &>/dev/null; then
        echo "sudo dnf install gcc make flex bison bc elfutils-libelf-devel openssl-devel"
    elif command -v pacman &>/dev/null; then
        echo "sudo pacman -S base-devel flex bison bc"
    elif command -v apk &>/dev/null; then
        echo "sudo apk add gcc make musl-dev flex bison bc elfutils-dev openssl-dev linux-headers"
    elif command -v zypper &>/dev/null; then
        echo "sudo zypper install gcc make flex bison bc libelf-devel libopenssl-devel"
    else
        echo "(install gcc, make, flex, bison, bc, libelf-dev, libssl-dev using your package manager)"
    fi
}

check_dep() {
    if ! command -v "$1" &>/dev/null; then
        echo "ERROR: '$1' is required but not found."
        return 1
    fi
}

echo "Checking build dependencies..."
MISSING=0
check_dep gcc || MISSING=1
check_dep make || MISSING=1
check_dep flex || MISSING=1
check_dep bison || MISSING=1
check_dep bc || MISSING=1

if [[ $MISSING -ne 0 ]]; then
    echo ""
    echo "Install missing dependencies:"
    echo "  $(detect_install_hint)"
    exit 1
fi

# Check for libelf headers (needed for CONFIG_OBJTOOL)
if [[ ! -f /usr/include/libelf.h ]] && [[ ! -f /usr/include/gelf.h ]]; then
    echo "WARNING: libelf headers not found."
    echo "  $(detect_install_hint)"
fi

echo "All dependencies found."
echo ""

# ---- Check disk space ----

AVAIL_KB=$(df -k "${BUILD_DIR%/*}" 2>/dev/null | awk 'NR==2{print $4}' || echo 0)
NEED_KB=$((5 * 1024 * 1024))  # 5GB
if [[ "$AVAIL_KB" -gt 0 ]] && [[ "$AVAIL_KB" -lt "$NEED_KB" ]]; then
    AVAIL_MB=$(( AVAIL_KB / 1024 ))
    echo "WARNING: Only ${AVAIL_MB}MB available in $(dirname "${BUILD_DIR}")."
    echo "  Kernel build needs ~5GB. Consider using --build-dir to change location."
fi

# ---- Download kernel source ----

KERNEL_URL="https://cdn.kernel.org/pub/linux/kernel/v${KERNEL_MAJOR}.x/linux-${KERNEL_VERSION}.tar.xz"
KERNEL_TARBALL="${BUILD_DIR}/linux-${KERNEL_VERSION}.tar.xz"
KERNEL_SRC="${BUILD_DIR}/linux-${KERNEL_VERSION}"

mkdir -p "${BUILD_DIR}"

if [[ -d "${KERNEL_SRC}" ]]; then
    echo "Kernel source already exists at ${KERNEL_SRC}"
else
    if [[ -f "${KERNEL_TARBALL}" ]]; then
        echo "Tarball already downloaded."
    else
        echo "Downloading linux-${KERNEL_VERSION}..."
        curl -L --progress-bar -o "${KERNEL_TARBALL}" "${KERNEL_URL}"
    fi

    echo "Extracting..."
    tar -xf "${KERNEL_TARBALL}" -C "${BUILD_DIR}"
fi

echo ""

# ---- Generate kernel config ----

echo "Generating minimal kernel config..."
cat > "${KERNEL_SRC}/.config" << 'KCONFIG'
#
# Minimal kernel config for chvm microVMs (Cloud Hypervisor)
# All required drivers built-in, no modules.
#

# General
CONFIG_LOCALVERSION="-chvm"
CONFIG_DEFAULT_HOSTNAME="chvm"
CONFIG_SYSVIPC=y
CONFIG_POSIX_MQUEUE=y
CONFIG_NO_HZ_IDLE=y
CONFIG_HIGH_RES_TIMERS=y
CONFIG_BPF_SYSCALL=y
CONFIG_CGROUPS=y
CONFIG_CGROUP_CPUACCT=y
CONFIG_CGROUP_DEVICE=y
CONFIG_CGROUP_FREEZER=y
CONFIG_CGROUP_SCHED=y
CONFIG_CGROUP_PIDS=y
CONFIG_CPUSETS=y
CONFIG_MEMCG=y
CONFIG_NAMESPACES=y
CONFIG_USER_NS=y
CONFIG_PID_NS=y
CONFIG_NET_NS=y
CONFIG_MULTIUSER=y
CONFIG_SYSFS=y
CONFIG_PROC_FS=y
CONFIG_TMPFS=y
CONFIG_TMPFS_POSIX_ACL=y
CONFIG_FUTEX=y
CONFIG_EPOLL=y
CONFIG_SIGNALFD=y
CONFIG_TIMERFD=y
CONFIG_EVENTFD=y
CONFIG_AIO=y
CONFIG_ADVISE_SYSCALLS=y
CONFIG_PRINTK=y
CONFIG_BUG=y
CONFIG_ELF_CORE=y
CONFIG_BASE_FULL=y
CONFIG_INOTIFY_USER=y

# No modules -- everything built-in
CONFIG_MODULES=n

# Processor
CONFIG_SMP=y
CONFIG_HYPERVISOR_GUEST=y
CONFIG_PARAVIRT=y
CONFIG_KVM_GUEST=y
CONFIG_X86_64=y
CONFIG_64BIT=y
CONFIG_PVH=y

# Memory
CONFIG_COMPACTION=y
CONFIG_MIGRATION=y
CONFIG_TRANSPARENT_HUGEPAGE=y

# Networking
CONFIG_NET=y
CONFIG_NETDEVICES=y
CONFIG_PACKET=y
CONFIG_UNIX=y
CONFIG_INET=y
CONFIG_IP_MULTICAST=y
CONFIG_IP_ADVANCED_ROUTER=y
CONFIG_IP_MULTIPLE_TABLES=y
CONFIG_IP_ROUTE_MULTIPATH=y
CONFIG_TCP_CONG_CUBIC=y
CONFIG_IPV6=y
CONFIG_BRIDGE=y
CONFIG_VLAN_8021Q=y
CONFIG_NETFILTER=y
CONFIG_NF_CONNTRACK=y
CONFIG_NF_NAT=y
CONFIG_NETFILTER_XTABLES=y
CONFIG_IP_NF_IPTABLES=y
CONFIG_IP_NF_FILTER=y
CONFIG_IP_NF_NAT=y
CONFIG_IP_NF_MANGLE=y
CONFIG_IP6_NF_IPTABLES=y
CONFIG_IP6_NF_FILTER=y

# TUN/TAP (for mycelium)
CONFIG_TUN=y

# Block devices
CONFIG_BLOCK=y
CONFIG_BLK_DEV=y
CONFIG_VIRTIO_BLK=y

# SCSI (needed for some virtio configs)
CONFIG_SCSI=y
CONFIG_BLK_DEV_SD=y

# Virtio (Cloud Hypervisor)
CONFIG_VIRTIO=y
CONFIG_VIRTIO_PCI=y
CONFIG_VIRTIO_PCI_MODERN=y
CONFIG_VIRTIO_BALLOON=y
CONFIG_VIRTIO_NET=y
CONFIG_VIRTIO_CONSOLE=y
CONFIG_VIRTIO_INPUT=y
CONFIG_VIRTIO_MMIO=y
CONFIG_HW_RANDOM_VIRTIO=y

# Vsock
CONFIG_VSOCKETS=y
CONFIG_VIRTIO_VSOCKETS=y

# VirtioFS / FUSE
CONFIG_FUSE_FS=y
CONFIG_VIRTIO_FS=y

# Filesystems
CONFIG_EXT4_FS=y
CONFIG_EXT4_USE_FOR_EXT2=y
CONFIG_EXT4_FS_POSIX_ACL=y
CONFIG_EXT4_FS_SECURITY=y
CONFIG_OVERLAY_FS=y
CONFIG_DEVTMPFS=y
CONFIG_DEVTMPFS_MOUNT=y

# Pseudo filesystems
CONFIG_PROC_FS=y
CONFIG_PROC_SYSCTL=y
CONFIG_SYSFS=y
CONFIG_TMPFS=y
CONFIG_HUGETLBFS=y

# Console / serial
CONFIG_TTY=y
CONFIG_VT=y
CONFIG_SERIAL_8250=y
CONFIG_SERIAL_8250_CONSOLE=y
CONFIG_HVC_DRIVER=y
CONFIG_HVC_CONSOLE=y

# Character devices
CONFIG_DEVMEM=y
CONFIG_HW_RANDOM=y

# Power management
CONFIG_ACPI=y
CONFIG_ACPI_BUTTON=y

# PCI
CONFIG_PCI=y
CONFIG_PCI_MSI=y
CONFIG_PCI_HOST_GENERIC=y

# Crypto (needed by networking)
CONFIG_CRYPTO=y
CONFIG_CRYPTO_AES=y
CONFIG_CRYPTO_SHA256=y
CONFIG_CRYPTO_SHA512=y
CONFIG_CRYPTO_HASH=y

# Disable unnecessary subsystems
# CONFIG_SOUND is not set
# CONFIG_DRM is not set
# CONFIG_USB_SUPPORT is not set
# CONFIG_WIRELESS is not set
# CONFIG_WLAN is not set
# CONFIG_INPUT_MOUSE is not set
# CONFIG_INPUT_TOUCHSCREEN is not set
# CONFIG_HWMON is not set
# CONFIG_THERMAL is not set
# CONFIG_AGP is not set
# CONFIG_PCMCIA is not set
# CONFIG_MEDIA_SUPPORT is not set
# CONFIG_RC_CORE is not set
# CONFIG_INFINIBAND is not set
# CONFIG_FIREWIRE is not set
# CONFIG_FIRMWARE_EDID is not set
# CONFIG_LOGO is not set
# CONFIG_FB is not set
# CONFIG_VGA_CONSOLE is not set
# CONFIG_BLUETOOTH is not set
# CONFIG_NFC is not set

# Embedded kernel config for detection
CONFIG_IKCONFIG=y
CONFIG_IKCONFIG_PROC=y
KCONFIG

echo "Config written."
echo ""

# ---- Apply GCC 15 compatibility patch ----

GCC_VERSION=$(gcc -dumpversion 2>/dev/null | cut -d. -f1)
if [[ "${GCC_VERSION}" -ge 15 ]] 2>/dev/null; then
    echo "Detected GCC ${GCC_VERSION} -- applying C23 compatibility patch..."

    # GCC 15+ treats bool/true/false as C keywords (C23).
    # Older kernels redefine them in include/linux/types.h which causes errors.
    TYPES_H="${KERNEL_SRC}/include/linux/types.h"
    if grep -q "^typedef _Bool.*bool;" "${TYPES_H}" 2>/dev/null; then
        sed -i 's/^typedef _Bool\t\tbool;/#ifndef __bool_true_false_are_defined\ntypedef _Bool\t\tbool;\n#endif/' "${TYPES_H}"
        echo "  Patched include/linux/types.h"
    fi

    STDBOOL_H="${KERNEL_SRC}/include/linux/stddef.h"
    if grep -q "^enum {$" "${STDBOOL_H}" 2>/dev/null; then
        # Wrap the false/true enum in a guard
        sed -i '/^enum {$/,/^};/{
            s/^enum {$/#ifndef __bool_true_false_are_defined\nenum {/
            s/^};$/};\n#endif/
        }' "${STDBOOL_H}"
        echo "  Patched include/linux/stddef.h"
    fi

    echo ""
fi

# ---- Build kernel ----

echo "Running make olddefconfig..."
make -C "${KERNEL_SRC}" olddefconfig -j"${JOBS}" > /dev/null 2>&1

echo "Building vmlinux (this may take 5-15 minutes)..."
make -C "${KERNEL_SRC}" vmlinux -j"${JOBS}" 2>&1 | tail -5

if [[ ! -f "${KERNEL_SRC}/vmlinux" ]]; then
    echo "ERROR: vmlinux not found after build. Check build output above."
    exit 1
fi

echo ""
echo "Build complete!"

# ---- Strip and install ----

VMLINUX_SIZE=$(wc -c < "${KERNEL_SRC}/vmlinux")
echo "  vmlinux size (unstripped): $(( VMLINUX_SIZE / 1048576 ))MB"

echo "Stripping debug symbols..."
strip --strip-debug "${KERNEL_SRC}/vmlinux"

VMLINUX_SIZE=$(wc -c < "${KERNEL_SRC}/vmlinux")
echo "  vmlinux size (stripped): $(( VMLINUX_SIZE / 1048576 ))MB"

mkdir -p "${OUTPUT_DIR}"
cp "${KERNEL_SRC}/vmlinux" "${OUTPUT_DIR}/${OUTPUT_NAME}"
chmod 644 "${OUTPUT_DIR}/${OUTPUT_NAME}"

echo ""
echo "=== Installed: ${OUTPUT_DIR}/${OUTPUT_NAME} ==="
echo ""
echo "To use this kernel:"
echo "  chvm kernel set-default ${OUTPUT_NAME}"
echo ""
echo "Or run directly:"
echo "  sudo chvm run --kernel ${OUTPUT_DIR}/${OUTPUT_NAME} --rm alpine:latest echo hello"
echo ""
