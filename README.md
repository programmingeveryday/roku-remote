# 📺 Roku Remote (Rust)

A fast, lightweight, and cross-platform native desktop remote control for Roku streaming sticks and Smart TVs. Written in **Rust** using **egui / eframe**.

---

## ✨ Features

- **Automatic Device Discovery**: Uses UPnP / SSDP UDP multicast (`M-SEARCH roku:ecp`) and network probes to automatically locate Roku devices on your local Wi-Fi / LAN.
- **Full Navigation D-Pad**: `Up`, `Down`, `Left`, `Right`, `OK / Select`, `Home`, `Back`, `Instant Replay`, and `Options / Info (*)`.
- **Media & Volume Control**: `Play / Pause`, `Fast Forward`, `Rewind`, `Volume Up`, `Volume Down`, and `Mute`.
- **App Launcher**: Instant one-click launch for popular channels and apps (YouTube, Netflix, Disney+, Prime Video, Hulu, Max, Spotify, Apple TV, Plex, etc.).
- **Live Active App Detection**: Automatically queries and displays which app is currently open on your TV.
- **Cross-Platform**: Compiles into a single standalone binary for **Linux (Wayland & X11)**, **macOS**, and **Windows**.

---

## ⚠️ Important: Roku Device Setup & Network Permissions

For this remote control (or any external app) to communicate with your Roku device over your local Wi-Fi, you **must ensure external control permissions are enabled** on your Roku:

1. Turn on your Roku device / TV.
2. Go to **Settings** → **System** → **Advanced system settings**.
3. Select **Control by mobile apps** → **Network access**.
4. Set it to **Default** or **Permissive** (do **not** leave it on *Disabled*).
5. Ensure your computer and your Roku device are connected to the **same local Wi-Fi / LAN network**.

> **Note:** Roku devices communicate via HTTP and SSDP over port `8060`. If the network access setting is Disabled, the Roku will block button presses and app launch requests.

---

## 🛠️ Prerequisites

You need a working Rust toolchain (version 1.75+ recommended):

```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
```

### Linux Build Dependencies
On Arch Linux / Omarchy:
```bash
sudo pacman -S base-devel pkg-config openssl
```

On Ubuntu / Debian:
```bash
sudo apt update
sudo apt install build-essential pkg-config libssl-dev libx11-dev libxkbcommon-dev libwayland-dev
```

---

## 🚀 Easy Installation Scripts

Automated compile-and-install scripts are provided in the `scripts/` directory for each operating system:

### 🐧 Linux
Compiles the optimized release binary, installs it to `~/.local/bin/roku-remote-rs`, and registers the `.desktop` menu launcher:
```bash
chmod +x scripts/install-linux.sh
./scripts/install-linux.sh
```

### 🍏 macOS
Compiles the release binary, creates a native `Roku Remote.app` bundle in `~/Applications` with High-DPI support, and adds the command-line binary to `~/.local/bin`:
```bash
chmod +x scripts/install-macos.sh
./scripts/install-macos.sh
```

### 🪟 Windows
Compiles `roku-remote-rs.exe`, installs it to `%LOCALAPPDATA%\Programs\RokuRemote`, adds it to the user's `PATH`, and creates a Start Menu shortcut:
```powershell
powershell -ExecutionPolicy Bypass -File .\scripts\install-windows.ps1
```

---

## 🔨 Manual Building & Running

### 1. Run in Development Mode
```bash
cargo run
```

### 2. Build Release Binary
```bash
cargo build --release
```
The optimized executable will be located at:
```bash
target/release/roku-remote-rs
```

---

## 🏗️ Cross-Compiling from Omarchy Linux

On **Omarchy Linux** (Arch Linux-based), you can easily cross-compile release binaries for **Windows** and **macOS** using native toolchains or cargo-cross.

### 🪟 Cross-Compiling for Windows (`x86_64-pc-windows-gnu`)

Omarchy provides the MinGW-w64 toolchain in the official Arch repositories:

1. **Install MinGW cross-compiler and Windows Rust target**:
   ```bash
   sudo pacman -S mingw-w64-gcc
   rustup target add x86_64-pc-windows-gnu
   ```

2. **Configure Cargo linker for MinGW**:
   Add the linker to your project or global cargo config (`~/.cargo/config.toml`):
   ```toml
   [target.x86_64-pc-windows-gnu]
   linker = "x86_64-w64-mingw32-gcc"
   ar = "x86_64-w64-mingw32-ar"
   ```

3. **Build the Windows executable**:
   ```bash
   cargo build --target x86_64-pc-windows-gnu --release
   ```
   The compiled `.exe` will be located at:
   ```bash
   target/x86_64-pc-windows-gnu/release/roku-remote-rs.exe
   ```

---

### 🍏 Cross-Compiling for macOS (Apple Silicon & Intel)

Because macOS builds require Apple SDK frameworks, the cleanest method on Linux is using [`cross`](https://github.com/cross-rs/cross) (which uses containerized environments via Podman or Docker):

1. **Install Podman (or Docker) and `cross`**:
   ```bash
   sudo pacman -S podman
   cargo install cross --git https://github.com/cross-rs/cross
   ```

2. **Build for Apple Silicon (M1/M2/M3/M4)**:
   ```bash
   cross build --target aarch64-apple-darwin --release
   ```

3. **Build for Intel Mac**:
   ```bash
   cross build --target x86_64-apple-darwin --release
   ```
   The resulting binary will be in `target/aarch64-apple-darwin/release/roku-remote-rs` and can be placed directly into `Roku Remote.app/Contents/MacOS/`.

---

### ⚡ Using `cross` for Zero-Setup Multi-Platform Builds

If you prefer not to install individual host C cross-compilers on Omarchy:
```bash
cargo install cross --git https://github.com/cross-rs/cross

# Build for Windows:
cross build --target x86_64-pc-windows-gnu --release

# Build for Linux musl (portable static binary):
cross build --target x86_64-unknown-linux-musl --release
```

---

## 📄 License

MIT or Apache-2.0.
