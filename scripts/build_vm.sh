#!/bin/bash
# scripts/build_vm.sh
# Downloads base Debian cloud image and customizes it to pre-install DebAI and Ollama

set -e

IMAGE_URL="https://cloud.debian.org/images/cloud/bookworm/latest/debian-12-generic-amd64.qcow2"
BASE_IMAGE="target/debian-12-base.qcow2"
VM_IMAGE="debai-debian-12.qcow2"

# 1. Ensure target directory exists
mkdir -p target

# 2. Build the deb package first
if [ ! -f target/debai.deb ]; then
    echo "==> Building debai.deb package..."
    ./scripts/build_deb.sh
fi

# 3. Download base image if not exists
if [ ! -f "$BASE_IMAGE" ]; then
    echo "==> Downloading Debian 12 base cloud image..."
    wget -O "$BASE_IMAGE" "$IMAGE_URL" || curl -Lo "$BASE_IMAGE" "$IMAGE_URL"
fi

# 4. Copy base image to VM target
echo "==> Creating VM image copy..."
cp "$BASE_IMAGE" "$VM_IMAGE"

# 5. Resize VM image to give space for Ollama models (15GB total)
echo "==> Resizing VM image to 15G..."
qemu-img resize "$VM_IMAGE" 15G

# 6. Generate first-boot provisioning script
echo "==> Creating first-boot setup scripts..."
cat << 'EOF' > target/debai-firstboot.sh
#!/bin/bash
# debai-firstboot.sh
# Runs on VM first boot to configure packages with active QEMU network access
set -e

echo "============================================="
echo "==> STARTING DEBAI FIRST-BOOT PROVISIONING <=="
echo "============================================="

# Wait for internet connectivity
echo "==> Waiting for network online..."
until curl -s --connect-timeout 3 https://deb.debian.org/ >/dev/null; do
    sleep 2
done

# Fix any broken dependency status and install dependencies
apt-get update
apt-get install -f -y
apt-get install -y jq git

# Install Ollama
echo "==> Installing Ollama..."
curl -fsSL https://ollama.com/install.sh | sh

# Pull target SLM
echo "==> Starting Ollama service and pulling qwen2.5:1.5b..."
systemctl start ollama || true
sleep 5
HOME=/usr/share/ollama ollama pull qwen2.5:1.5b

# Start and enable the DebAI daemon now that all dependencies are installed
echo "==> Enabling and starting DebAI Daemon (aid)..."
systemctl enable aid.service || true
systemctl restart aid.service || true

# Cleanup first-boot triggers
echo "==> Cleaning up first-boot hooks..."
systemctl disable debai-firstboot.service
rm -f /etc/systemd/system/multi-user.target.wants/debai-firstboot.service
rm -f /lib/systemd/system/debai-firstboot.service
rm -f /usr/local/bin/debai-firstboot.sh

echo "============================================="
echo "==> DEBAI FIRST-BOOT SETUP COMPLETED!     <=="
echo "============================================="
EOF

# 7. Generate first-boot service unit
cat << 'EOF' > target/debai-firstboot.service
[Unit]
Description=DebAI First Boot Setup
After=network-online.target
Wants=network-online.target

[Service]
Type=oneshot
ExecStart=/usr/local/bin/debai-firstboot.sh
StandardOutput=journal+console
StandardError=journal+console
RemainAfterExit=yes

[Install]
WantedBy=multi-user.target
EOF

# 8. Generate systemd DHCP configuration file
cat << 'EOF' > target/80-dhcp.network
[Match]
Name=en* eth* ens*

[Network]
DHCP=yes
EOF

# 9. Customize the VM image (runs offline; all network operations deferred to first boot)
echo "==> Customizing VM image using virt-customize (requires sudo for boot/kernel access)..."
sudo virt-customize -a "$VM_IMAGE" \
  --root-password password:debai \
  --run-command "useradd -m -s /bin/bash -p \$(openssl passwd -1 debai) debai" \
  --run-command "usermod -aG sudo debai" \
  --run-command "echo 'debai ALL=(ALL) NOPASSWD:ALL' >> /etc/sudoers" \
  --upload target/80-dhcp.network:/etc/systemd/network/80-dhcp.network \
  --upload target/debai.deb:/tmp/debai.deb \
  --run-command "dpkg -i /tmp/debai.deb || true" \
  --upload target/debai-firstboot.sh:/usr/local/bin/debai-firstboot.sh \
  --run-command "chmod 755 /usr/local/bin/debai-firstboot.sh" \
  --upload target/debai-firstboot.service:/lib/systemd/system/debai-firstboot.service \
  --run-command "ln -sf /lib/systemd/system/debai-firstboot.service /etc/systemd/system/multi-user.target.wants/debai-firstboot.service" \
  --run-command "rm -f /tmp/debai.deb"

# Clean up temp files
rm -f target/debai-firstboot.sh target/debai-firstboot.service target/80-dhcp.network

# Restore host user ownership on the output image file
sudo chown $(id -u):$(id -g) "$VM_IMAGE"

echo "==> VM image '$VM_IMAGE' created and customized successfully!"
echo "==> Credentials inside VM:"
echo "    username: debai"
echo "    password: debai"
echo "    root password: debai"
echo "==> You can now run the VM with: ./scripts/run_vm.sh"
