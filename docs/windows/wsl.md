# Installing WSL on Windows ARM64

## Overview

WSL2 (Windows Subsystem for Linux) is required for running Linux containers
and tools on Windows. This documents the installation steps and known
limitations discovered while setting up a Windows ARM64 development
environment.

## Prerequisites

- Windows 11 ARM64 (build 26100+)
- Administrator access
- **Hardware virtualization support** — WSL2 requires Hyper-V, which needs
  bare-metal access to CPU virtualization extensions

## Installation Steps

### 1. Enable required Windows features

Open an **elevated PowerShell** (Run as Administrator):

```powershell
# Enable WSL feature
dism.exe /online /enable-feature /featurename:Microsoft-Windows-Subsystem-Linux /all /norestart

# Enable Virtual Machine Platform (required for WSL2)
dism.exe /online /enable-feature /featurename:VirtualMachinePlatform /all /norestart

# Enable Hypervisor Platform
dism.exe /online /enable-feature /featurename:HypervisorPlatform /all /norestart
```

### 2. Reboot

```powershell
shutdown /r /t 0
```

### 3. Install the WSL package

The inbox `wsl.exe` on Windows 11 is a stub — it cannot self-install. Use
winget to install the actual WSL runtime:

```powershell
winget install Microsoft.WSL --source winget --accept-source-agreements --accept-package-agreements
```

This downloads the WSL MSI from GitHub (e.g. `wsl.2.6.3.0.arm64.msi`) and
installs it.

### 4. Install a Linux distribution

```powershell
# List available distros
wsl --list --online

# Install Ubuntu 24.04
wsl --install Ubuntu-24.04
```

On first launch you will be prompted to create a Linux user account.

### 5. Verify

```powershell
wsl --status
wsl -l -v
```

## Known Limitation: No Nested Virtualization on Apple Silicon VMs

**WSL2 does not work inside a QEMU/UTM virtual machine on Apple Silicon.**

WSL2 requires Hyper-V, which requires hardware virtualization extensions.
Apple's Hypervisor.framework (HVF) does not support nested virtualization on
ARM64. When running Windows ARM64 in UTM/QEMU with `-accel hvf`, the guest
sees:

```
Hyper-V Requirements: A hypervisor has been detected. Features required for
Hyper-V will not be displayed.
```

And attempting to register a WSL distro fails with:

```
Error code: Wsl/InstallDistro/Service/RegisterDistro/CreateVm/HCS/HCS_E_HYPERV_NOT_INSTALLED
```

### Workarounds

- **Use bare-metal Windows ARM64 hardware** (e.g. Surface Pro, Lenovo X13s,
  or a Windows Dev Kit) where Hyper-V can access the CPU directly.
- **Use Docker Desktop with WSL2 backend** on bare-metal — Docker Desktop
  installs and manages WSL automatically.
- **Use Parallels Desktop** instead of UTM — Parallels supports nested
  virtualization on Apple Silicon (macOS 15+), which may allow WSL2 to work.

## Steps We Performed (for reference)

On a Windows 11 ARM64 VM (UTM/QEMU on macOS Apple Silicon):

1. Enabled `Microsoft-Windows-Subsystem-Linux` via DISM — succeeded
2. Enabled `VirtualMachinePlatform` via DISM — succeeded
3. Rebooted
4. Installed `Microsoft.WSL` via winget — succeeded (v2.6.3 ARM64 MSI)
5. Enabled `HypervisorPlatform` via DISM — succeeded
6. Rebooted
7. Attempted `wsl --install Ubuntu-24.04` — **failed** with
   `HCS_E_HYPERV_NOT_INSTALLED`

The failure is due to the Apple Silicon nested virtualization limitation
described above. All software prerequisites are in place; the only missing
piece is hardware-level Hyper-V support.
