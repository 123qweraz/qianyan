# Qianyan IME

一款 Rust 编写的拼音输入法,支持 Linux (evdev/IBus/Wayland) 和 Windows (TSF)。

## 运行

### Linux

#### 方式一: 使用预编译压缩包 (推荐)

```bash
# 解压
tar xzf qianyan-ime-linux-x86_64.tar.gz
cd qianyan-ime

# 直接运行 (便携模式)
./qianyan-ime

# 或安装到系统 (自动配置权限)
sudo ./install.sh
# 重新登录后即可使用
qianyan-ime
```

#### 方式二: 从源码编译

```bash
# 编译
cargo build --release

# 安装
sudo cp target/release/qianyan-ime /usr/local/bin/
sudo cp qianyan-ime.desktop /usr/share/applications/

# 运行
qianyan-ime
```

#### 权限配置 (evdev 模式必须)

evdev 模式需要读取 `/dev/input/event*` 和写入 `/dev/uinput` 的权限:

```bash
# 1. 将当前用户加入 input 组
sudo usermod -aG input $USER

# 2. 创建 udev 规则，允许 input 组访问 uinput
echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/99-qianyan-ime-uinput.rules

# 3. 重载 udev 规则
sudo udevadm control --reload-rules
sudo udevadm trigger

# 4. 如果 /dev/uinput 权限未即时生效:
sudo chmod 660 /dev/uinput
sudo chgrp input /dev/uinput
```

> **重要**: 加入 input 组后 **必须注销并重新登录** 才能生效。

#### 后端选择

程序自动检测可用后端，优先级: evdev → IBus → Wayland。也可手动指定:

```bash
qianyan-ime --backend=evdev     # 强制 evdev (硬件拦截，性能最佳)
qianyan-ime --backend=ibus      # 强制 IBus
qianyan-ime --backend=wayland   # 强制 Wayland 原生协议
```

### Windows

使用 Visual Studio 打开,或:
```powershell
.\scripts\release\make_windows_release.ps1
```

## 开发

```bash
# 克隆
git clone https://github.com/123qweraz/qianyan-ime.git
cd qianyan-ime

# 开发模式编译
cargo build

# 运行
cargo run

# 测试
cargo test
```

## 项目结构

```
qianyan_ime/
├── Cargo.toml          # Workspace 配置
├── crates/
│   ├── core/          # 核心类型和配置
│   ├── engine/        # 输入法引擎
│   ├── ui/           # UI (Slint)
│   ├── platform-linux/   # Linux 平台代码
│   └── platform-windows/ # Windows 平台代码
├── src/               # 主程序入口
├── configs/           # 配置文件
├── dicts/             # 词库 (JSON)
└── scripts/           # 安装和发布脚本
```

## 配置

配置文件位于 `configs/`,修改后重启生效。

## 依赖

- Rust 1.70+
- Linux: libevdev, wayland-client
- Windows: Visual Studio (TSF)

## License

MIT
