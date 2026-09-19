#!/usr/bin/env bash
#
# 千言输入法 - 绿色版（免 root）安装脚本
#
# 把程序与 data/dicts/configs 复制到同一个用户目录，使其成为自包含的便携安装：
#   - 程序与数据/配置同目录（find_project_root / get_config_dir 依赖该布局）
#   - 注册用户级应用菜单项，可选开机自启
#   - 在 KDE Wayland 下注册虚拟键盘输入法（kwinrc + wayland launcher）
#
# 用法:
#   ./install-portable.sh [--prefix DIR] [--autostart] [--no-kde-vk]
#
#   --prefix DIR   安装目录（默认: ~/.local/share/qianyan-ime）
#   --autostart    同时写入 ~/.config/autostart 开机自启
#   --no-kde-vk    跳过 KDE 虚拟键盘注册
#
# 卸载:
#   rm -rf "$PREFIX"
#   rm -f ~/.local/share/applications/qianyan-ime.desktop
#   rm -f ~/.local/share/applications/qianyan-ime-wayland-launcher.desktop
#   rm -f ~/.config/autostart/qianyan-ime.desktop
#   并清除 ~/.config/kwinrc 中 [Wayland] InputMethod（可用系统设置改回）

set -euo pipefail

PREFIX="${QIANYAN_PREFIX:-$HOME/.local/share/qianyan-ime}"
AUTOSTART=0
KDE_VK=1

usage() {
    sed -n '2,22p' "$0" | sed 's/^# \{0,1\}//'
}

while [ $# -gt 0 ]; do
    case "$1" in
        --prefix) PREFIX="$2"; shift 2 ;;
        --prefix=*) PREFIX="${1#*=}"; shift ;;
        --autostart) AUTOSTART=1; shift ;;
        --no-kde-vk) KDE_VK=0; shift ;;
        -h|--help) usage; exit 0 ;;
        *) echo "未知参数: $1" >&2; usage >&2; exit 1 ;;
    esac
done

SCRIPT_DIR="$(cd "$(dirname "$(readlink -f "$0")")" && pwd)"

# 定位源码/资源根目录，兼容发布包与源码仓库两种布局
ROOT=""
for cand in "$SCRIPT_DIR" "$SCRIPT_DIR/.." "$SCRIPT_DIR/../.."; do
    if [ -d "$cand/dicts" ] || [ -d "$cand/data" ]; then
        ROOT="$(cd "$cand" && pwd)"
        break
    fi
done
if [ -z "$ROOT" ]; then
    echo "错误: 在 $SCRIPT_DIR 附近找不到 data/dicts 资源目录" >&2
    exit 1
fi

# 定位二进制目录
if [ -x "$ROOT/qianyan-ime" ]; then
    BIN_DIR="$ROOT"
elif [ -x "$ROOT/target/release/qianyan-ime" ]; then
    BIN_DIR="$ROOT/target/release"
else
    echo "错误: 找不到 qianyan-ime 可执行文件（请先 cargo build --release）" >&2
    exit 1
fi

echo "== 千言输入法 绿色版安装 =="
echo "资源目录: $ROOT"
echo "二进制:   $BIN_DIR"
echo "安装到:   $PREFIX"
echo

mkdir -p "$PREFIX"

# 1. 可执行文件
for b in qianyan-ime qianyan-ime-gui qianyan-web-settings; do
    if [ -f "$BIN_DIR/$b" ]; then
        install -m755 "$BIN_DIR/$b" "$PREFIX/$b"
    else
        echo "警告: 缺少 $b，跳过" >&2
    fi
done

# 2. 资源目录（覆盖更新）
for d in data dicts picture sounds; do
    if [ -d "$ROOT/$d" ]; then
        mkdir -p "$PREFIX/$d"
        cp -a "$ROOT/$d/." "$PREFIX/$d/"
    fi
done

# 3. 配置：首次安装写入默认，已存在则保留用户配置
mkdir -p "$PREFIX/configs"
if [ -d "$ROOT/configs" ]; then
    for f in "$ROOT"/configs/*; do
        [ -e "$f" ] || continue
        bn="$(basename "$f")"
        [ "$bn" = ".config.lock" ] && continue
        if [ ! -e "$PREFIX/configs/$bn" ]; then
            cp -a "$f" "$PREFIX/configs/$bn"
        fi
    done
fi

# 4. 用户级应用菜单项
APPS_DIR="$HOME/.local/share/applications"
mkdir -p "$APPS_DIR"
APP_DESKTOP="$APPS_DIR/qianyan-ime.desktop"
cat > "$APP_DESKTOP" <<EOF
[Desktop Entry]
Name=Qianyan-IME
Comment=千言输入法
Exec="$PREFIX/qianyan-ime"
Path=$PREFIX
Icon=qianyan-ime
Terminal=false
Type=Application
Categories=Settings;Utility;
Keywords=input;ime;pinyin;
StartupNotify=false
EOF
echo "已写入应用菜单项: $APP_DESKTOP"

# 5. 图标
ICON_DIR="$HOME/.local/share/icons/hicolor/256x256/apps"
if [ -f "$ROOT/picture/qianyan-ime_v2.png" ]; then
    mkdir -p "$ICON_DIR"
    cp -f "$ROOT/picture/qianyan-ime_v2.png" "$ICON_DIR/qianyan-ime.png"
fi

# 6. 开机自启（可选）
if [ "$AUTOSTART" = 1 ]; then
    AUTOSTART_DIR="$HOME/.config/autostart"
    mkdir -p "$AUTOSTART_DIR"
    cp -f "$APP_DESKTOP" "$AUTOSTART_DIR/qianyan-ime.desktop"
    echo "已启用开机自启: $AUTOSTART_DIR/qianyan-ime.desktop"
fi

# 7. KDE Wayland 虚拟键盘注册
if [ "$KDE_VK" = 1 ]; then
    if printf ':%s:' "${XDG_CURRENT_DESKTOP:-}" | grep -qi ':kde:'; then
        if "$PREFIX/qianyan-ime" --register-kde-vk; then
            :
        else
            echo "警告: KDE 虚拟键盘注册失败，稍后在 KDE 会话中运行 qianyan-ime 会自动重试" >&2
        fi
    else
        echo "提示: 当前非 KDE 会话，跳过虚拟键盘注册（登录 KDE 后首次运行会自动注册）"
    fi
fi

echo
echo "安装完成。"
echo "  配置中心: 运行 $PREFIX/qianyan-ime 后访问 http://127.0.0.1:18765"
echo "  KDE Wayland: 请注销并重新登录（或 kwin_wayland --replace）使输入法生效"
