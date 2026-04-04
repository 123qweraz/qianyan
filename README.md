# Shian IME

一款 Rust 编写的拼音输入法,支持 Linux (evdev/IBus/Wayland) 和 Windows (TSF)。

## 运行

### Linux

```bash
# 编译
cargo build --release

# 安装
sudo cp target/release/shian-ime /usr/local/bin/
sudo cp rust-ime.desktop /usr/share/applications/

# 运行
shian-ime
```

### Windows

使用 Visual Studio 打开,或:
```powershell
.\scripts\release\make_windows_release.ps1
```

## 开发

```bash
# 克隆
git clone https://github.com/123qweraz/shian.git
cd shian/shian_ime

# 开发模式编译
cargo build

# 运行
cargo run

# 测试
cargo test
```

## 项目结构

```
shian_ime/
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
