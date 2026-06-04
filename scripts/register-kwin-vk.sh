#!/bin/bash
set -e

EXE="$(dirname "$(readlink -f "$0")")/../target/release/qianyan-ime"
if [ ! -f "$EXE" ]; then
    EXE="$(which qianyan-ime 2>/dev/null || true)"
fi
if [ -z "$EXE" ] || [ ! -f "$EXE" ]; then
    echo "错误: 找不到 qianyan-ime 可执行文件"
    echo "用法: $0 [qianyan-ime 路径]"
    echo "或先编译: cargo build --release"
    exit 1
fi

APPS_DIR="$HOME/.local/share/applications"
mkdir -p "$APPS_DIR"

DESKTOP_FILE="$APPS_DIR/qianyan-ime-wayland-launcher.desktop"

cat > "$DESKTOP_FILE" << EOF
[Desktop Entry]
Name=Qianyan Input Method (Wayland)
Name[zh_CN]=千言输入法 (Wayland)
GenericName=Input Method
GenericName[zh_CN]=输入法
Comment=Qianyan Chinese Input Method Engine (KDE Virtual Keyboard)
Comment[zh_CN]=千言中文输入法引擎（KDE 虚拟键盘）
Exec=$EXE --backend=wayland
Icon=input-keyboard
Terminal=false
Type=Application
Categories=System;Utility;
StartupNotify=false
NoDisplay=true
OnlyShowIn=KDE
X-KDE-StartupNotify=false
X-KDE-Wayland-VirtualKeyboard=true
EOF

echo "[1/2] 已写入 $DESKTOP_FILE"

if command -v kwriteconfig6 &>/dev/null; then
    kwriteconfig6 --file kwinrc --group Wayland --key InputMethod qianyan-ime-wayland-launcher.desktop
    echo "[2/2] 已设置 kwinrc (kwriteconfig6)"
elif command -v kwriteconfig5 &>/dev/null; then
    kwriteconfig5 --file kwinrc --group Wayland --key InputMethod qianyan-ime-wayland-launcher.desktop
    echo "[2/2] 已设置 kwinrc (kwriteconfig5)"
else
    echo "[2/2] 警告: 未找到 kwriteconfig5/6，手动执行:"
    echo "  kwriteconfig6 --file kwinrc --group Wayland --key InputMethod qianyan-ime-wayland-launcher.desktop"
fi

if command -v qdbus6 &>/dev/null; then
    qdbus6 org.kde.KWin /KWin reconfigure 2>/dev/null || true
elif command -v qdbus &>/dev/null; then
    qdbus org.kde.KWin /KWin reconfigure 2>/dev/null || true
fi

echo ""
echo "完成！重新登入 Wayland 会话后生效。"
echo "或者执行: kwin_x11 --replace & kwin_wayland --replace  # 重启 KWin"
