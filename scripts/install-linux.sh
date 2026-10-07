#!/usr/bin/env bash
set -euo pipefail

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}=== Roku Remote (Rust) - Linux Build & Install ===${NC}"

# Check for Rust / Cargo
if ! command -v cargo &> /dev/null; then
    echo -e "${RED}Error: Cargo is not installed or not in PATH.${NC}"
    echo "Please install Rust using: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
    exit 1
fi

PROJECT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$PROJECT_DIR"

echo -e "${YELLOW}Building optimized release binary...${NC}"
cargo build --release

BIN_SOURCE="$PROJECT_DIR/target/release/roku-remote-rs"
if [[ ! -f "$BIN_SOURCE" ]]; then
    echo -e "${RED}Error: Build output $BIN_SOURCE not found.${NC}"
    exit 1
fi

# Target directories
INSTALL_DIR="${HOME}/.local/bin"
DESKTOP_DIR="${HOME}/.local/share/applications"
ICON_DIR="${HOME}/.local/share/icons/hicolor/128x128/apps"

mkdir -p "$INSTALL_DIR"
mkdir -p "$DESKTOP_DIR"
mkdir -p "$ICON_DIR"

echo -e "${YELLOW}Installing executable to ${INSTALL_DIR}...${NC}"
rm -f "$INSTALL_DIR/roku-remote-rs"
cp "$BIN_SOURCE" "$INSTALL_DIR/roku-remote-rs"
chmod +x "$INSTALL_DIR/roku-remote-rs"

# Install icon if available
if [[ -f "$PROJECT_DIR/assets/icon-128.png" ]]; then
    echo -e "${YELLOW}Installing application icon to ${ICON_DIR}...${NC}"
    cp "$PROJECT_DIR/assets/icon-128.png" "$ICON_DIR/org.omarchy.roku.remote.png"
fi

echo -e "${YELLOW}Creating desktop launcher in ${DESKTOP_DIR}...${NC}"
cat <<EOF > "$DESKTOP_DIR/org.omarchy.roku.remote.desktop"
[Desktop Entry]
Name=Roku Remote
Comment=Native Roku Remote Control with Omarchy Theming and Keyboard Shortcuts
Exec=${INSTALL_DIR}/roku-remote-rs
Icon=org.omarchy.roku.remote
Terminal=false
Type=Application
Categories=AudioVideo;Utility;Network;
StartupWMClass=org.omarchy.roku.remote
Keywords=roku;remote;tv;streaming;omarchy;
EOF

# Update desktop database if available
if command -v update-desktop-database &> /dev/null; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi

echo -e "${GREEN}✓ Successfully built and installed Roku Remote for Linux!${NC}"
echo -e "Binary path:  ${INSTALL_DIR}/roku-remote-rs"
echo -e "Desktop entry: ${DESKTOP_DIR}/org.omarchy.roku.remote.desktop"
echo -e "Run directly via: roku-remote-rs"
