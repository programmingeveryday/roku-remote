#!/usr/bin/env bash
set -euo pipefail

# Colors
GREEN='\033[0;32m'
BLUE='\033[0;34m'
YELLOW='\033[1;33m'
RED='\033[0;31m'
NC='\033[0m'

echo -e "${BLUE}=== Roku Remote (Rust) - macOS Build & Install ===${NC}"

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

# 1. Install CLI binary in /usr/local/bin or ~/.local/bin
INSTALL_DIR="${HOME}/.local/bin"
mkdir -p "$INSTALL_DIR"
cp -f "$BIN_SOURCE" "$INSTALL_DIR/roku-remote-rs"
chmod +x "$INSTALL_DIR/roku-remote-rs"

# 2. Package as a standard macOS .app bundle in ~/Applications
APP_NAME="Roku Remote.app"
APP_DIR="${HOME}/Applications/${APP_NAME}"
MACOS_DIR="${APP_DIR}/Contents/MacOS"
RESOURCES_DIR="${APP_DIR}/Contents/Resources"

echo -e "${YELLOW}Packaging macOS Application Bundle at ${APP_DIR}...${NC}"
rm -rf "$APP_DIR"
mkdir -p "$MACOS_DIR"
mkdir -p "$RESOURCES_DIR"

# Copy binary into app bundle
cp -f "$BIN_SOURCE" "$MACOS_DIR/roku-remote-rs"
chmod +x "$MACOS_DIR/roku-remote-rs"

# Generate Info.plist
cat <<EOF > "${APP_DIR}/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>roku-remote-rs</string>
    <key>CFBundleIdentifier</key>
    <string>org.omarchy.roku.remote</string>
    <key>CFBundleName</key>
    <string>Roku Remote</string>
    <key>CFBundleDisplayName</key>
    <string>Roku Remote</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleVersion</key>
    <string>1</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
EOF

echo -e "${GREEN}✓ Successfully built and installed Roku Remote for macOS!${NC}"
echo -e "Application: ${APP_DIR}"
echo -e "CLI Binary:  ${INSTALL_DIR}/roku-remote-rs"
echo -e "You can open it from Spotlight, Finder (~/Applications), or terminal with 'open ~/Applications/\"Roku Remote.app\"'"
