# 👁️ Qianyan IME

> **"Thousands of words, starting at your fingertips."**  
> A cross-platform Rust-powered input method built for the ultimate typing experience.

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](https://opensource.org/licenses/MIT)
[![Build Status](https://img.shields.io/badge/build-passing-brightgreen.svg)]()
[![Platform: Linux/Windows](https://img.shields.io/badge/platform-Linux%20%7C%20Windows-blue.svg)]()

---

## 🌟 Why Qianyan?

Qianyan is more than just a tool; it's a rethink of efficient input.

### 1. 🚀 Blazing Performance (Rust Powered)
Built with Rust for minimal memory footprint and near-zero latency. Supports **evdev hardware-level interception** on Linux, bypassing traditional middleware for a truly raw performance.

### 2. 🎯 Innovative SBSRF 3-Code Stroke System
- **3-Code Precision**: First 2 strokes + Last 2 strokes + Pinyin Initial.
- **Memorable**: A custom stroke-to-letter matrix that turns radicals into simple key combinations.
- **Filtering**: Apply auxiliary codes after Pinyin to pinpoint the exact character instantly.

### 3. 🌐 Cross-Platform & Backend Support
- **Linux**: Supports evdev, IBus, and native Wayland protocols.
- **Windows**: Deeply integrated with the TSF (Text Services Framework).

### 4. 🛠️ Visual Web Config Center
Configure your IME directly in your browser: hot-reload changes, rich presets (Xiaohe, Sogou), and real-time style previews.

---

## 📸 At a Glance

| Intelligent Pinyin | Web Config Center | Practice Mode |
| :---: | :---: | :---: |
| ![Typing Demo](readme_picture/rust-ime1.png) | ![Config Panel](readme_picture/webconfig.png) | ![Practice](readme_picture/style.png) |

---

## 🔥 Key Features

- **Versatile Modes**: Full Pinyin, Double Pinyin, Jianpin, and Stroke input.
- **🎯 English Aux Code**: Pinyin + English prefix filtering (e.g., `pingguo` + `app` $\rightarrow$ `苹果`).
- **⚡ Long Final Shortcuts**: Quick mappings for complex finals like `iang`, `uang`, etc.
- **⌨️ Vim-style Nav**: Innovative CapsLock navigation mode. Use `H/J/K/L` without leaving the home row.
- **💎 Dictionary Editor**: Web-based graphical tool for CRUD operations, weight adjustment, and batch imports.
- **🧠 Word Discovery**: Smart algorithms that automatically extract high-frequency new words from your typing.
- **Smart Learning**: N-Gram language modeling + User dictionaries that adapt to your style.
- **Intelligent Punctuation**: Extended punctuation via Long-press/Double-tap and auto-pairing.
- **Rare Character Support**: Large-scale dictionary covering even the most obscure Hanzi.

---

## 🛠️ Installation & Running

### Linux Installation

#### Method 1: Pre-compiled Package (Recommended)
```bash
# Extract and enter directory
tar xzf qianyan-ime-linux-x86_64.tar.gz && cd qianyan-ime

# Install and automatically configure permissions
sudo ./install.sh
# Run
qianyan-ime
```

#### Method 2: From Source
```bash
# Install dependencies (Ubuntu/Debian)
sudo apt install rustc cargo libevdev-dev libdbus-1-dev clang

# Build and Install
cargo build --release
sudo cp target/release/qianyan-ime /usr/local/bin/
```

#### Permissions (Required for evdev)
1. Add user to `input` group: `sudo usermod -aG input $USER`
2. Create udev rule:
   `echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' | sudo tee /etc/udev/rules.d/99-qianyan-ime-uinput.rules`
3. Reload: `sudo udevadm control --reload-rules && sudo udevadm trigger`
4. **Log out and log back in** to apply changes.

---

## 🏗️ Project Architecture (Engineering)

The project uses a **Rust Workspace** architecture:

```
qianyan-ime/
├── src/main.rs             # Main Entry Point
├── crates/
│   ├── core/               # Core types & config definitions
│   ├── engine/             # IME Engine (FSM, Pipeline, Schemes)
│   ├── ui/                 # UI (Slint GUI, Web Server)
│   ├── platform-linux/     # Linux Backend (evdev/IBus)
│   └── platform-windows/   # Windows Backend (TSF)
├── configs/                # Runtime JSON configurations
├── dicts/                  # Raw Dictionaries (JSON)
└── static/                 # Web Config assets
```

### Core Crate Highlights
- **`engine`**: Handles pinyin matching, N-Gram learning, and SBSRF stroke processing.
- **`ui`**: Axum-powered Web server on `localhost:18765` and Slint for the desktop UI.
- **`platform-*`**: Manages low-level key interception and injection per OS.

---

## 📚 Dictionary Structure

- `dicts/chinese/chars/`: Basic characters, split into Levels 1-3.
- `dicts/chinese/words/`: Main vocabulary, High-freq words, Emojis.
- `dicts/stroke/`: Index data specifically for stroke-based input.
- `data/`: Compiled FST (Finite State Transducer) indices for millisecond-speed lookups.

---

## ❓ FAQ

**Q: Keyboard not responding after start?**
A: Check if your user is in the `input` group. Run `groups` to verify.

**Q: How to switch Double Pinyin schemes?**
A: Visit `http://localhost:18765`, choose your scheme under "Double Pinyin Settings," and click Save to apply immediately.

**Q: Candidate window position is wrong?**
A: Desktop environments (GNOME/KDE/Hyprland) vary in positioning support. Using the Wayland backend is recommended.

---

## 🤝 Contributing
This project is open-source under the [MIT License](LICENSE). [Issues](https://github.com/123qweraz/qianyan-ime/issues) and Pull Requests are welcome!

---
> **Crafted with care by Shian, guarding every keystroke with Rust.**
