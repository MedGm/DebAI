#!/bin/bash
# scripts/build_deb.sh
# Automates the creation of the debai.deb Debian package

set -e

echo "==> Compiling DebAI inside a Debian Bookworm Docker container to match target GLIBC..."
docker run --rm -v "$(pwd)":/usr/src/debai -w /usr/src/debai rust:bookworm cargo build --release

# Setup package directory structure
PKG_DIR="target/debai-pkg"
rm -rf "$PKG_DIR"
mkdir -p "$PKG_DIR/DEBIAN"
mkdir -p "$PKG_DIR/usr/local/bin"
mkdir -p "$PKG_DIR/etc/debai"
mkdir -p "$PKG_DIR/lib/systemd/system"
mkdir -p "$PKG_DIR/usr/share/debai/plugins"

# Copy binaries and scripts
cp target/release/aid "$PKG_DIR/usr/local/bin/aid"
cp target/release/aiterm "$PKG_DIR/usr/local/bin/aiterm"
cp scripts/sandbox.sh "$PKG_DIR/usr/local/bin/debai-sandbox"

# Copy policy configurations
cp debai_policy.json "$PKG_DIR/etc/debai/policy.json"

# Copy systemd service unit
cp installer/aid.service "$PKG_DIR/lib/systemd/system/aid.service"

# Copy plugins
cp plugins/audit_logger.py "$PKG_DIR/usr/share/debai/plugins/audit_logger.py"
cp plugins/safety_sentinel.py "$PKG_DIR/usr/share/debai/plugins/safety_sentinel.py"

# Make binaries, scripts, and plugins executable
chmod 755 "$PKG_DIR/usr/local/bin/aid"
chmod 755 "$PKG_DIR/usr/local/bin/aiterm"
chmod 755 "$PKG_DIR/usr/local/bin/debai-sandbox"
chmod 755 "$PKG_DIR/usr/share/debai/plugins/audit_logger.py"
chmod 755 "$PKG_DIR/usr/share/debai/plugins/safety_sentinel.py"

# Write DEBIAN/control file
cat << 'EOF' > "$PKG_DIR/DEBIAN/control"
Package: debai
Version: 1.0.0
Section: admin
Priority: optional
Architecture: amd64
Maintainer: DebAI Team <info@debai.org>
Depends: libc6, python3, jq
Description: DebAI - Safe, Sandboxed AI Terminal and OS Control Daemon
 Provides natural language OS interaction, automated plan generation, OverlayFS sandboxing, and customizable security policy layers.
EOF

# Write DEBIAN/postinst script
cat << 'EOF' > "$PKG_DIR/DEBIAN/postinst"
#!/bin/sh
set -e
if [ "$1" = "configure" ]; then
    echo "==> Configuring DebAI package..."
    systemctl daemon-reload || true
    systemctl enable aid.service || true
    systemctl restart aid.service || true
fi
EOF
chmod 755 "$PKG_DIR/DEBIAN/postinst"

# Write DEBIAN/prerm script
cat << 'EOF' > "$PKG_DIR/DEBIAN/prerm"
#!/bin/sh
set -e
if [ "$1" = "remove" ] || [ "$1" = "deconfigure" ]; then
    echo "==> Stopping DebAI service before removal..."
    systemctl stop aid.service || true
    systemctl disable aid.service || true
fi
EOF
chmod 755 "$PKG_DIR/DEBIAN/prerm"

# Build the Debian package
echo "==> Building Debian package debai.deb..."
dpkg-deb --build "$PKG_DIR" target/debai.deb

echo "==> Debian package generated successfully at target/debai.deb!"
