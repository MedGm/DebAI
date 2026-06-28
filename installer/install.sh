#!/bin/bash
set -e

# DebAI Installer Script

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

echo -e "${GREEN}==> Building DebAI Workspace in release mode...${NC}"
cargo build --release

echo -e "${GREEN}==> Installing binaries to /usr/local/bin/...${NC}"
sudo cp target/release/aid /usr/local/bin/aid
sudo cp target/release/aiterm /usr/local/bin/aiterm

echo -e "${GREEN}==> Setting permissions on binaries...${NC}"
sudo chmod 755 /usr/local/bin/aid
sudo chmod 755 /usr/local/bin/aiterm

echo -e "${GREEN}==> Installing systemd service unit...${NC}"
sudo cp installer/aid.service /etc/systemd/system/aid.service
sudo chmod 644 /etc/systemd/system/aid.service

echo -e "${GREEN}==> Reloading systemd manager configuration...${NC}"
sudo systemctl daemon-reload

echo -e "${GREEN}==> Enabling and starting aid service...${NC}"
sudo systemctl enable aid.service
sudo systemctl restart aid.service

echo -e "${GREEN}==> Verifying service status...${NC}"
if systemctl is-active --quiet aid.service; then
    echo -e "${GREEN}==> DebAI Daemon (aid) is active and running successfully!${NC}"
    echo -e "Unix Socket path: /run/debai/aid.sock"
    echo -e "You can run queries using: ${GREEN}aiterm --socket /run/debai/aid.sock explain \"ls -la\"${NC}"
else
    echo -e "${RED}Error: DebAI Daemon failed to start. Run 'journalctl -u aid.service' for logs.${NC}"
    exit 1
fi
