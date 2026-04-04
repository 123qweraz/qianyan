# 🚀 rust-IME Easy Installation Guide

Welcome to rust-IME! If this is your first time installing software on Linux/Windows, don't worry—just follow these steps.

---

## Linux Installation

### Step 1: Extract Files
1. Find the downloaded `rust-ime-linux-x64.tar.gz` file.
2. Right-click the file and select **"Extract Here"**.
3. Enter the extracted folder.

### Step 2: Open Terminal
1. Right-click on an empty space within the folder.
2. Select **"Open in Terminal"**.

### Step 3: Run Install Script
1. In the terminal, type (or copy-paste) the following command:
   ```bash
   bash ./install.sh
   ```
2. Press **Enter**.
3. **Enter Password**: You might be prompted for your password. Note that no characters will appear while typing; this is normal. Just type it and press Enter.

### Step 4: Restart (Crucial!)
Once finished, the terminal will say "Installation Complete!".
**You MUST restart your computer or log out and back in**, otherwise the input method may not be able to access your keyboard.

---

## Windows Installation

### Step 1: Extract ZIP
1. Extract the `rust-ime-windows-v0.1.0.zip` file.
2. Open the extracted folder.

### Step 2: Run Installer
1. Right-click `install.bat` and select **"Run as Administrator"**.
2. Follow the on-screen instructions.

---

## ❓ Troubleshooting

### 0. Desktop Environment Support (Linux)
- **Best Support**: KDE (Wayland), COSMIC, Hyprland.
- **Possible Issues**: GNOME or XFCE (UI might not float correctly).
- **Core Functionality**: Typing usually works even if the UI fails to appear, provided permissions are correct.

### 1. "Permission denied"
If `install.sh` fails to run, try:
```bash
chmod +x install.sh
```

### 2. Configuration Page
Enter `http://localhost:8765` in your browser to access settings. Ensure `rust-ime` is running.

### 3. Keyboard Unresponsive
This usually means permissions haven't taken effect.
- **Fix**: Ensure you have restarted your computer. If it still doesn't work, try running `sudo ./install.sh` again.

### 4. Uninstallation
To remove the application:
```bash
# Linux
sudo rm /usr/local/bin/rust-ime
# Windows
Run uninstall.bat as Administrator
```

---
💡 **Tip**: This project is still in development. If you encounter bugs, please report them on GitHub.
