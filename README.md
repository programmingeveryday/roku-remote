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

## 📱 Roku Configuration Note

Roku devices use the **External Control Protocol (ECP)** over port `8060`. 

If your Roku is running newer firmware with restricted network permissions:
1. On your Roku device, navigate to:  
   **Settings → System → Advanced system settings → Control by mobile apps → Network access**
2. Set it to **Default** or **Permissive**.

---

## 🏗️ Cross-Compiling

Because this project uses `egui` and `eframe`, you can compile it for other operating systems:

- **Windows (x86_64)**:
  ```bash
  cargo build --target x86_64-pc-windows-gnu --release
  ```
- **macOS (Apple Silicon)**:
  ```bash
  cargo build --target aarch64-apple-darwin --release
  ```

---

## 📄 License

MIT or Apache-2.0.
