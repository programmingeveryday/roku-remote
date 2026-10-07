# 📺 Roku Remote (Rust)

A modern, fast, lightweight, and cross-platform native desktop remote control for Roku streaming sticks and Smart TVs. Written in **Rust** using **egui / eframe**.

![Roku Remote](docs/images/screenshot-wide.png)

---

## ✨ Features

- **Multi-Device Discovery & Selector**: Automatically discovers all Roku devices on your local Wi-Fi / LAN via SSDP UDP multicast and parallel multi-threaded subnet probes. Includes an interactive dropdown selector to switch between multiple Rokus across different rooms, as well as a manual IP entry mode.
- **One-Click Refresh with Live Spinner Feedback**: Dedicated **`🔄`** refresh button right beside the device selector that triggers network discovery, polls media playback state, queries active applications, and refreshes installed channel inventory with animated spinner feedback.
- **In-App Setup Guide & Permission Alerts**: Built-in interactive setup guide modal (`⚙ Setup`) with guidance for HDMI-CEC setup and automatic warning banners if a Roku's Mobile App Control is in "Limited" mode.
- **Smart Power Control & HDMI-CEC 1-Touch Play**:
  - Unified single-button power toggle (`Power Off` in red / `Power On` in green) reflecting TV and display status.
  - Automatically sends HDMI-CEC 1-Touch Play sequences (`Home` + `Power`) on power up so Roku Streaming Sticks wake connected televisions and switch to the correct HDMI input seamlessly.
  - Independently monitors device connectivity (`Online` status in Device Details).
- **Full Navigation D-Pad**: Vector-rendered arrow keys (`▲`, `▼`, `◄`, `►`), `OK / Select`, `Home`, `Back`, `Instant Replay`, and `Options / Info (*)`.
- **Media & Volume Control**: Dedicated playback row (`<<`, `Play / Pause`, `>>`), `Volume Up`, `Volume Down`, and `Mute`.
- **Live Playback State & Dynamic Play/Pause Button**: Real-time status indicator (`▶ Playing`, `⏸ Paused`, `⏳ Buffering`) paired with a context-aware Play/Pause button that dynamically changes label and color (green for playing, amber for paused).
- **True Sleep State & Zero Idle Overhead**:
  - Automatically enters deep sleep mode when minimized (instant), unfocused (3s), or idle (10s), stopping all background network queries and file polling (0% CPU, 0 network requests).
  - Wakes up instantly on user interaction and updates status.
- **Quick Launch Apps with High-Res Channel Icons**:
  - Automatically queries and displays all installed channels on the active Roku device.
  - Automatically rescans apps whenever a new device is selected or on first connection (cleanly clearing stale channel cards if none are found).
  - Full-bleed app cards displaying high-res icons (supports both PNG and JPEG formats, including Netflix, Prime Video, YouTube, Disney+, Hulu, HBO Max, etc.).
  - Seamless fallback to channel names if an icon is not available.
- **Offline & Instant Startup Icon Caching**:
  - Channel list and decoded icons are cached locally (`~/.cache/roku-remote-rs/icons/`), enabling instant rendering on startup without waiting for network queries.
  - Dedicated **`🔄 Refresh Apps`** button (and shortcut) to rescan and update installed channels anytime.
- **Adaptive Responsive Layout**:
  - Dynamically detects window dimensions and reflows into wide (two-column) or narrow (single-column) views with zero text collisions or button crowding.
  - Remote controls vertically align with the top of application cards in wide view.
  - Device Info and Setup Guide automatically adapt between centered floating dialogs in wide mode and dedicated full-width in-page views in narrow mode.
- **Persistent Window Size & Position**:
  - Automatically restores your last-used window position and dimensions across launches.
- **Full Keyboard Control & Shortcut Cheat Sheet**:
  - Direct keyboard control for navigation, volume, playback, and app refresh.
  - Built-in shortcuts cheat sheet modal (`Ctrl + ,` or `Esc` to close).
- **Omarchy / System Desktop Theme Integration**:
  - Automatically detects and matches Omarchy system themes (`colors.toml`) with live hot-reloading.
- **Device Details View**:
  - Inspect model name, model number, software version, Wi-Fi network, UI resolution, power mode, IP address, and live/sleeping App Status.
- **Modular & Idiomatic Rust Architecture**:
  - Refactored into clean modules (`app`, `models`, `roku::client`, `roku::parser`, `theme`) with 100% unit test coverage for XML parsers and color utilities.

---

## 📱 Responsive Adaptive Layout

Roku Remote dynamically detects window width and automatically reflows its user interface:

* **Wide Screen Mode (window width ≥ 680px)**: 
  Splits into two side-by-side columns. Remote navigation and media playback controls remain pinned on the left (`300px`), vertically aligned with the channel cards, while the responsive grid of quick-launch channel cards expands across the right side. Dialog boxes display as centered modal windows.
* **Compact / Narrow Window Mode (window width < 680px)**: 
  Automatically transitions into an uncrowded single-column layout:
  - **Header Bar**: App brand and status indicator on the left; clean action buttons (`Device Info`, `Setup`, `Power Toggle`) on the right.
  - **Row 2**: Device selector dropdown with margin separation on the left, quick action toolbar on the right.
  - **Row 3**: Real-time media playback state and discovery / scan status.
  - **Control & App Stack**: Ergonomic remote control pad followed by vertically scrollable quick-launch channel cards. Ideal for tiling window managers (Hyprland, Sway, i3) or sidebars.
  - **Responsive In-Page Dialogs**: Device Details, Setup Guide, and Shortcuts cleanly render in-page with single-click `Close` buttons and auto-wrapping text down to 260px.

<p align="center">
  <img src="docs/images/screenshot-narrow.png" width="340" alt="Roku Remote Compact View" />
  <br/>
  <em>Automatic single-column reflow when window is resized below 680px</em>
</p>

---

## ⌨️ Keyboard Shortcuts

| Shortcut | Action | Description |
| :--- | :--- | :--- |
| **Arrow Keys** | Navigation | Move Up, Down, Left, Right |
| **Enter** / **Space** | OK / Select | Activate selected item |
| **Backspace** / **Esc** | Back | Back / Return button |
| **H** | Home | Return to Roku Home screen |
| **P** | Play / Pause | Toggle media playback |
| **R** | Replay | Instant replay (jumps back ~10s) |
| **O** / **\*** | Options / Star | Roku Options / Asterisk (`*`) menu |
| **Ctrl + Up / Down** | Volume | Volume Up / Volume Down |
| **M** | Mute | Toggle audio mute |
| **I** | Device Info | Toggle Roku Device Details / Info dialog |
| **S** | Setup Guide | Toggle Setup & Troubleshooting Guide |
| **Ctrl + Shift + M** / **Ctrl + M** | Window Mode | Toggle compact Always-on-Top / Maximized Normal |
| **Ctrl + Shift + R** or **Ctrl + A** | Refresh Apps | Check and reload installed apps & icons |
| **Ctrl + ,** | Cheat Sheet | Toggle keyboard shortcuts modal |

---

## ⚠️ Important: Roku Device Setup & Network Permissions

For this remote control (or any external app) to communicate with your Roku device over your local Wi-Fi, you **must ensure external control permissions are enabled** on your Roku:

1. Turn on your Roku device / TV.
2. Go to **Settings** → **System** → **Advanced system settings**.
3. Select **Control by mobile apps** → **Network access**.
4. Set it to **Default** or **Permissive** (do **not** leave it on *Disabled*).
5. Ensure your computer and your Roku device are connected to the **same local Wi-Fi / LAN network**.

> **Note:** Roku devices communicate via HTTP and SSDP over port `8060`. If the network access setting is Disabled, the Roku will block button presses and app launch requests.

### 📺 TV Power Control via Roku (HDMI-CEC 1-Touch Play)

If using a **Roku Streaming Stick** plugged into an external TV:
1. **On your Roku**: Navigate to **Settings** → **System** → **Control other devices (CEC)** → Check **1-touch play**.
2. **On your TV**: Ensure HDMI-CEC is enabled in your TV's settings menu (often named *Anynet+* on Samsung, *Bravia Sync* on Sony, *SimpLink* on LG, or *CEC*).
3. When you click **`Power On`**, Roku Remote issues an HDMI-CEC 1-Touch Play wake command (`Home` + `Power`), turning on the television screen and automatically switching to the Roku's HDMI input.

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

## 🔨 Manual Building & Testing

### 1. Run Tests
```bash
cargo test
```

### 2. Run in Development Mode
```bash
cargo run
```

### 3. Build Release Binary
```bash
cargo build --release
```
The optimized executable will be located at `target/release/roku-remote-rs`.

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
