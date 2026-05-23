# 千言输入法 (Qianyan IME) 说明文档

一款使用 Rust 编写的跨平台拼音输入法，支持 Linux (evdev/IBus/Wayland) 和 Windows (TSF)。

---

## 目录

- [项目简介](#项目简介)
- [项目结构详解](#项目结构详解)
- [安装方法](#安装方法)
  - [Linux 安装](#linux-安装)
  - [Windows 安装](#windows-安装)
- [配置说明](#配置说明)
- [功能特性](#功能特性)
- [开发指南](#开发指南)
- [常见问题](#常见问题)

---

## 项目简介

| 属性 | 说明 |
|------|------|
| **项目名称** | 千言输入法 (Qianyan IME) |
| **编程语言** | Rust (Edition 2021) |
| **许可证** | MIT |
| **版本** | 0.1.0 |
| **作者** | Shian |

**支持平台**：
- **Linux**: evdev (硬件拦截)、IBus、Wayland 原生协议
- **Windows**: TSF (Text Services Framework)

---

## 项目结构详解

### 整体架构

项目采用 **Rust 工作区 (Workspace)** 架构，代码分为 5 个逻辑 crate + 1 个主入口：

```
qianyan-ime/
├── src/main.rs                    # 程序主入口
├── Cargo.toml                     # 工作区根配置
│
├── crates/
│   ├── core/                      # 核心类型和配置定义
│   ├── engine/                    # 输入法核心引擎
│   ├── ui/                        # 用户界面 (Slint + Web)
│   ├── platform-linux/            # Linux 平台实现
│   └── platform-windows/          # Windows 平台实现
│
├── configs/                       # 运行时配置文件 (JSON)
├── dicts/                         # 原始词库数据
├── data/                          # 编译后的词库索引
├── static/                        # Web 配置页面资源
├── tests/                         # 测试脚本
├── tools/                         # 词典处理工具
└── scripts/                       # 安装和发布脚本
```

### Crate 详细说明

#### 1. `crates/core/` - 核心类型

| 文件 | 功能 |
|------|------|
| `lib.rs` | Rect 结构体、InputMethodHost trait |
| `config.rs` | Config 主配置结构体 (783行)，包含所有配置子结构 |
| `utils.rs` | 工具函数：项目根查找、标点加载、音节加载 |

**职责**：零依赖的基础类型和配置定义，被所有其他 crate 依赖。

#### 2. `crates/engine/` - 输入法引擎

| 文件/模块 | 功能 |
|-----------|------|
| `compiler.rs` | 词库编译器，将 JSON 词典编译为 FST 索引 |
| `compositor.rs` | 组合器，管理输入状态 |
| `config_manager.rs` | 配置管理器，热重载支持 |
| `context.rs` | 引擎上下文，共享状态 |
| `dispatcher.rs` | 按键调度器 (Command/InputEvent/KeyDispatcher) |
| `keys.rs` | 按键定义和映射 |
| `pipeline.rs` | 搜索管道，候选词匹配和排序 |
| `scheme.rs` | 输入方案 trait 定义 |
| `session.rs` | 输入会话管理 |
| `sound.rs` | 声音管理 (按键音、错误音) |
| `trie.rs` | Trie 树索引，FST 封装 |
| `user_data.rs` | 用户数据管理 (自学习词库) |

**processor/ 子模块**：

| 文件 | 功能 |
|------|------|
| `commands.rs` | 命令处理 (切换中英文、翻页等) |
| `fsm.rs` | 有限状态机，管理输入状态转换 |
| `handlers.rs` | 按键处理器 |
| `intents.rs` | 意图识别和处理 |
| `learning.rs` | 学习引擎 (N-Gram、词频调整) |
| `punctuation.rs` | 标点符号处理 |
| `session_state.rs` | 会话状态管理 |

**schemes/ 输入方案**：

| 文件 | 功能 |
|------|------|
| `chinese.rs` | 中文输入方案 (全拼、双拼) |
| `english.rs` | 英文输入方案 |
| `japanese.rs` | 日文输入方案 (罗马字转假名) |
| `stroke.rs` | 笔画输入方案 |

#### 3. `crates/ui/` - 用户界面

| 文件 | 功能 |
|------|------|
| `lib.rs` | UI 事件定义、AppState、CandidateDisplay trait |
| `main.slint` | Slint UI 主界面定义 |
| `candidate.slint` | 候选词组件 |
| `status_bar.slint` | 状态栏组件 |
| `gui_slint.rs` | Slint GUI 实现 |
| `slint_window.rs` | Slint 窗口封装 |
| `tray.rs` | 系统托盘 (KSNI on Linux) |
| `web.rs` | Web 配置服务器 (Axum, 端口 18765) |
| `linux_notify.rs` | Linux 桌面通知 |

**Web 配置页面** (端口 18765)：

```
static/
├── index.html              # 首页
├── help.html               # 帮助页
├── virtual_keyboard.html   # 虚拟键盘
├── css/style.css           # 样式表
├── js/settings.js          # 配置脚本
├── presets/                # 预设方案
│   ├── 小鹤双拼.json
│   ├── 搜狗双拼.json
│   ├── 自然码双拼.json
│   └── 布局方案/
└── settings/               # 14个配置页面
    ├── appearance.html     # 外观设置
    ├── dictionary.html     # 词库设置
    ├── double_pinyin.html  # 双拼设置
    ├── fuzzy.html          # 模糊音设置
    ├── hotkeys.html        # 快捷键设置
    └── ...
```

#### 4. `crates/platform-linux/` - Linux 平台

| 文件 | 功能 |
|------|------|
| `cli.rs` | 命令行参数解析 (--backend, --daemon 等) |
| `runtime.rs` | 运行时初始化，创建输入主机 |

**hosts/ 后端实现**：

| 文件 | 功能 |
|------|------|
| `traits.rs` | 后端 trait 定义 (InputMethodHost) |
| `evdev_host.rs` | evdev 后端 - 直接读取输入设备 |
| `vkbd.rs` | 虚拟键盘 - uinput 模拟输入 |

#### 5. `crates/platform-windows/` - Windows 平台

| 文件 | 功能 |
|------|------|
| `lib.rs` | DLL 入口 (DllMain、DllGetClassObject) |
| `main.rs` | EXE 入口 |
| `constants.rs` | IME GUID 常量 |
| `class_factory.rs` | COM Class Factory |
| `registry.rs` | 注册表操作 (注册/注销 TSF) |
| `text_service.rs` | TSF 文本服务实现 |
| `tsf.rs` | TSF 框架封装 |

### 数据目录结构

#### dicts/ - 原始词库

```
dicts/
├── chinese/
│   ├── chars/              # 单字数据
│   │   ├── chars.json      # 基础字库
│   │   ├── level2.json     # 二级字库
│   │   └── level3.json     # 三级字库
│   ├── words/              # 词汇数据
│   │   ├── words.json      # 主词库
│   │   ├── new_words.json  # 新词
│   │   ├── emoji.json      # Emoji
│   │   ├── high_freq.json  # 高频词
│   │   ├── low_freq.json   # 低频词
│   │   └── proper_nouns/   # 专有名词
│   ├── punctuation.json    # 标点符号定义
│   ├── syllables.txt       # 拼音音节表
│   ├── syllable_freq.txt   # 音节频率
│   └── basic_tokens.txt    # 基础词元
├── english/                # 英文词库
├── japanese/               # 日文词库 (N1-N5, kana)
├── stroke/                 # 笔画输入数据
└── shengpizi/              # 生僻字词库
```

#### data/ - 编译后索引

```
data/
├── chinese/
│   ├── trie.index          # FST 索引文件
│   └── trie.data           # FST 数据文件
├── english/
├── japanese/
├── stroke/
└── shengpizi/
```

#### configs/ - 运行时配置

| 文件 | 功能 |
|------|------|
| `input.json` | 核心输入配置 (597行) |
| `appearance.json` | 外观配置 (字体、颜色、主题) |
| `hotkeys.json` | 快捷键配置 |
| `files.json` | 文件路径配置 |
| `linux.json` | Linux 平台特有配置 |

---

## 安装方法

### Linux 安装

#### 方式一：使用预编译包 (推荐)

```bash
# 解压
tar xzf qianyan-ime-linux-x86_64.tar.gz
cd qianyan-ime

# 方式 A: 便携运行 (无需安装)
./qianyan-ime

# 方式 B: 系统安装 (自动配置权限)
sudo ./install.sh
# 重启或注销后即可使用
qianyan-ime
```

#### 方式二：从源码编译

**依赖安装**：

```bash
# Ubuntu/Debian
sudo apt install rustc cargo libevdev-dev libdbus-1-dev clang

# Fedora
sudo dnf install rust cargo libevdev-devel dbus-devel clang

# Arch
sudo pacman -S rust cargo libevdev dbus clang
```

**编译安装**：

```bash
# 克隆源码
git clone https://github.com/123qweraz/qianyan-ime.git
cd qianyan-ime

# 编译
cargo build --release

# 安装
sudo cp target/release/qianyan-ime /usr/local/bin/
sudo cp qianyan-ime.desktop /usr/share/applications/
```

#### 权限配置 (evdev 模式必需)

evdev 模式需要读取 `/dev/input/event*` 和写入 `/dev/uinput` 的权限：

```bash
# 1. 将当前用户加入 input 组
sudo usermod -aG input $USER

# 2. 创建 udev 规则
echo 'KERNEL=="uinput", GROUP="input", MODE="0660", OPTIONS+="static_node=uinput"' \
  | sudo tee /etc/udev/rules.d/99-qianyan-ime-uinput.rules

# 3. 重载 udev 规则
sudo udevadm control --reload-rules
sudo udevadm trigger

# 4. 如果权限未即时生效
sudo chmod 660 /dev/uinput
sudo chgrp input /dev/uinput
```

> **重要**: 加入 input 组后**必须注销并重新登录**才能生效。

#### 后端选择

程序自动检测可用后端，优先级: **evdev → IBus → Wayland**。

也可手动指定：

```bash
qianyan-ime --backend=evdev     # 强制 evdev (硬件拦截，性能最佳)
qianyan-ime --backend=ibus      # 强制 IBus
qianyan-ime --backend=wayland   # 强制 Wayland 原生协议
```

#### 命令行参数

| 参数 | 说明 |
|------|------|
| `--backend=<backend>` | 指定后端: evdev, ibus, wayland |
| `--daemon` | 后台守护进程运行 |
| `--no-daemon` | 前台运行 |
| `--list` | 列出可用后端 |
| `--help` | 显示帮助 |

### Windows 安装

#### 方式一：使用安装包

运行 `qianyan-ime-setup.exe`，按照向导完成安装。

#### 方式二：从源码编译

**需求**：
- Visual Studio 2022 (含 C++ 开发工具)
- Rust 工具链

```powershell
# 编译
cargo build --release

# 注册 TSF 组件 (管理员权限)
cd target/release
regsvr32 qianyan-ime.dll
```

---

## 配置说明

### Web 配置界面

运行输入法后，在浏览器打开：

```
http://localhost:18765
```

提供 14 个配置页面：

| 页面 | 功能 |
|------|------|
| 外观设置 | 字体、颜色、主题、候选窗样式 |
| 词库设置 | 启用/禁用词库、词库权重 |
| 双拼设置 | 方案选择 (小鹤/搜狗/自然码)、自定义映射 |
| 模糊音设置 | z/zh, c/ch, s/sh, n/l, 前后鼻音等 |
| 快捷键设置 | 中英文切换、翻页、候选选择等 |
| 标点设置 | 标点映射、长按/双击扩展 |
| 高级设置 | 自学习、自动排序、N-Gram 等 |

### 核心配置项 (input.json)

#### 基础设置

```json
{
  "default_profile": "chinese",    // 默认输入方案
  "autostart": true,                // 开机自启
  "commit_mode": "single",          // 上屏模式: single/phrase
  "phantom_type": "Pinyin"          // 幻影显示类型
}
```

#### 双拼配置

```json
{
  "enable_double_pinyin": true,
  "double_pinyin_scheme": {
    "name": "小鹤双拼",
    "initials": {
      "u": "sh",
      "i": "ch",
      "v": "zh"
    },
    "rimes": {
      "t": "ue",
      "m": "ian",
      "x": "ia",
      "s": "ong",
      "b": "in",
      "c": "ao",
      "w": "ei",
      "y": "un",
      "r": "uan",
      "k": "ao",
      "l": "iang",
      "f": "en",
      "z": "ou",
      "j": "an",
      "q": "iu",
      "p": "ie",
      "d": "ai"
    }
  }
}
```

#### 模糊音配置

```json
{
  "enable_fuzzy_pinyin": true,
  "fuzzy_config": {
    "z_zh": true,      // z = zh
    "c_ch": true,      // c = ch
    "s_sh": true,      // s = sh
    "n_l": false,      // n = l
    "r_l": false,      // r = l
    "f_h": false,      // f = h
    "an_ang": false,   // an = ang
    "en_eng": false,   // en = eng
    "in_ing": false    // in = ing
  }
}
```

#### 排序权重

```json
{
  "ranking": {
    "length_penalty": 50000.0,        // 长度惩罚
    "user_dict_bonus": 10000000.0,    // 用户词库加成
    "exact_match_bonus": 10000000.0,  // 精确匹配加成
    "single_char_bonus": 1000000.0     // 单字加成
  }
}
```

---

## 功能特性

### 输入方案

| 方案 | 说明 |
|------|------|
| **中文** | 全拼、双拼 (小鹤/搜狗/自然码)、简拼、辅助码 |
| **英文** | 英文单词输入、自动补全 |
| **日文** | 罗马字转假名、汉字转换 |
| **笔画** | 横竖撇捺折 (hspnz) 输入 |

### 核心功能

1. **双拼支持**
   - 内置三种双拼方案
   - 支持自定义双拼映射
   - 辅助码支持

2. **模糊音**
   - z/zh, c/ch, s/sh
   - n/l, r/l, f/h
   - an/ang, en/eng, in/ing

3. **自学习**
   - 词频自动调整
   - 新词发现
   - N-Gram 语言模型
   - 用户词库持久化

4. **标点符号**
   - 中日文标点映射
   - 长按扩展标点
   - 双击扩展标点
   - 智能标点配对

5. **Web 配置**
   - 运行时可视化配置
   - 无需重启即时生效
   - 预设方案导入导出

### 快捷键

| 按键 | 功能 |
|------|------|
| `Shift` | 中英文切换 |
| `Ctrl + Space` | 启用/禁用输入法 |
| `-` / `=` | 翻页 |
| `,` / `.` | 翻页 (可配置) |
| `1-9` | 选择候选词 |
| `Space` | 选择第一个候选 |
| `Enter` | 直接上屏拼音 |
| `Esc` | 清除输入 |
| `Backspace` | 删除一个拼音 |

---

## 开发指南

### 环境设置

```bash
# 克隆项目
git clone https://github.com/123qweraz/qianyan-ime.git
cd qianyan-ime

# 安装依赖 (Linux)
sudo apt install libevdev-dev libdbus-1-dev clang

# 开发编译
cargo build

# 运行
cargo run

# 运行测试
cargo test

# 运行集成测试
bash tests/test_core_logic.sh
```

### 代码规范

项目使用：
- `rustfmt` 格式化代码
- `clippy` 代码检查 (`clippy.toml` 配置)
- `cargo-deny` 依赖安全检查 (`deny.toml`)

```bash
# 格式化
cargo fmt

# Clippy 检查
cargo clippy

# 依赖安全检查
cargo deny check
```

### 词典工具

`tools/` 目录包含 59 个 Python 脚本用于词典处理：

| 类别 | 脚本示例 |
|------|----------|
| 处理词典 | `add_stroke_aux.py`, `add_weights_to_words.py` |
| 分析检查 | `analyze_stroke_collisions.py`, `audit_dictionary.py` |
| 导出转换 | `export_multi_json.py`, `json_to_text.py` |
| 修复更新 | `fix_docs.py`, `update_char_weights.py` |

### 架构文档

详细架构重构计划参见：
```
docs/ARCHITECTURE_REFACTORING_PLAN.md
```

---

## 常见问题

### Q1: 启动后键盘没反应？

**原因**：evdev 模式权限未正确配置。

**解决**：
1. 确保用户已加入 `input` 组
2. 确保 `udev` 规则已创建
3. **注销并重新登录**

```bash
groups  # 确认包含 input
ls -l /dev/uinput  # 确认权限
```

### Q2: 候选窗不显示？

**原因**：Slint 渲染后端问题。

**解决**：
```bash
# Linux 默认使用 software 后端，可尝试切换
export SLINT_BACKEND=skia
qianyan-ime
```

**桌面环境兼容性**：
- **最佳**: KDE (Wayland), COSMIC, Hyprland
- **可能有问题**: GNOME, XFCE (候选窗悬浮可能不正常)

### Q3: Web 配置页面打不开？

**原因**：输入法未运行，或端口被占用。

**解决**：
1. 确认 `qianyan-ime` 进程在运行
2. 检查端口 18765 是否被占用

```bash
ps aux | grep qianyan-ime
netstat -tlnp | grep 18765
```

### Q4: 如何卸载？

**Linux**：
```bash
# 如果使用了 install.sh
sudo rm /usr/local/bin/qianyan-ime
sudo rm /usr/share/applications/qianyan-ime.desktop
sudo rm /etc/udev/rules.d/99-qianyan-ime-uinput.rules

# 删除用户数据
rm -rf ~/.local/share/qianyan-ime
```

**Windows**：
```powershell
# 管理员权限
regsvr32 /u qianyan-ime.dll
# 然后删除程序目录
```

### Q5: 如何查看日志？

```bash
# 前台运行查看日志
RUST_LOG=debug qianyan-ime --no-daemon

# 或
RUST_LOG=info cargo run
```

---

## License

MIT License

---

## 反馈

问题反馈：https://github.com/123qweraz/qianyan-ime/issues
