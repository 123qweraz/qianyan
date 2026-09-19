#!/bin/bash
#
# 注册 KDE Wayland 虚拟键盘输入法。
#
# 该脚本只是 qianyan-ime --register-kde-vk 的便捷封装：真正的注册逻辑在
# crates/platform-linux/src/kwin.rs，避免两边规则不一致（历史上这里曾用相对
# 短名写 kwinrc，KWin 无法解析，导致输入法静默不启动）。

set -e

EXE="${1:-}"

if [ -z "$EXE" ]; then
    SCRIPT_DIR="$(dirname "$(readlink -f "$0")")"
    if [ -x "$SCRIPT_DIR/../target/release/qianyan-ime" ]; then
        EXE="$SCRIPT_DIR/../target/release/qianyan-ime"
    else
        EXE="$(command -v qianyan-ime 2>/dev/null || true)"
    fi
fi

if [ -z "$EXE" ] || [ ! -x "$EXE" ]; then
    echo "错误: 找不到 qianyan-ime 可执行文件"
    echo "用法: $0 [qianyan-ime 路径]"
    echo "或先编译: cargo build --release"
    exit 1
fi

"$EXE" --register-kde-vk

echo ""
echo "完成！重新登入 Wayland 会话后生效。"
echo "或者执行: kwin_wayland --replace  # 重启 KWin"
