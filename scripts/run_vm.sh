#!/bin/bash
# scripts/run_vm.sh
# Runs the customized DebAI QEMU virtual machine

set -e

VM_IMAGE="debai-debian-12.qcow2"

if [ ! -f "$VM_IMAGE" ]; then
    echo "Error: VM image '$VM_IMAGE' not found. Please run ./scripts/build_vm.sh first."
    exit 1
fi

KVM_FLAG=""
if [ -c /dev/kvm ] && [ -r /dev/kvm ] && [ -w /dev/kvm ]; then
    KVM_FLAG="-enable-kvm"
    echo "==> KVM virtualization enabled."
else
    echo "==> Warning: KVM virtualization not available. Running in emulation mode (slower)..."
fi

echo "==> Starting DebAI QEMU VM..."
echo "==> SSH port forwarded: localhost:2222 -> VM:22"
echo "==> To access the VM serial console, press Ctrl+A, then X to exit QEMU."
echo "==> Booting..."

qemu-system-x86_64 \
  -m 2048 \
  -smp 2 \
  -drive file="$VM_IMAGE",format=qcow2,if=virtio \
  -net nic,model=virtio -net user,hostfwd=tcp::2222-:22 \
  -nographic \
  $KVM_FLAG
