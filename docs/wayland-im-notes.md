# Wayland 输入法协议开发笔记

## 架构概述

qianyan-ime 的 Linux 输入后端有三种模式：

| 后端 | 文件 | 原理 | 状态 |
|------|------|------|------|
| evdev | `evdev_host.rs` | 直接读 `/dev/input/event*`，通过 uinput 虚拟键盘 + 剪贴板粘贴输出 | 默认后端，X11/Wayland 通用 |
| Wayland v2 | `wayland_host.rs` | `zwp_input_method_v2` 协议，KDE/现代 compositor | 保留，依赖 compositor 支持 |
| Wayland v1 | `wayland_host_v1.rs` | `zwp_input_method_v1` 协议，weston/老 compositor | 保留，依赖 compositor 支持 |

运行时通过 `create_wayland_host()` 自动检测 compositor 提供的协议版本（v1/v2），命令行 `--backend=wayland` 强制使用。

## 关键区别：v1 vs v2 协议

| 特性 | v1 (`zwp_input_method_v1`) | v2 (`zwp_input_method_v2`) |
|------|---------------------------|---------------------------|
| 接口 | 直接 bind IM 对象，无 manager | `zwp_input_method_manager_v2` → `get_input_method()` |
| activate | 创建 `zwp_input_method_context_v1` | 直接在 IM 对象上激活 |
| grab_keyboard | 返回标准 `wl_keyboard`（有 keymap/key/modifiers 事件） | 返回 `zwp_input_method_keyboard_grab_v2` |
| **按键透传** | `context.key(serial, time, key, state)` — **有此能力** | **无** forward_key，只能用 `commit_string` 输出文本 |
| 文本输出 | `context.commit_string(serial, text)` | `im.commit_string(text)` + `im.commit(serial)` |
| preedit | `context.preedit_string(serial, text, commit)` | `im.set_preedit_string(text, begin, end)` + `im.commit(serial)` |
| serial 同步 | `context.commit_state(serial)` 事件 | `im.done` 事件（serial 自增） |
| delete_surrounding | `context.delete_surrounding_text(index, length)` | `im.delete_surrounding_text(before, after)` |

v1 的 `context.key()` 请求可以直接将未处理的按键透传给应用，而 v2 没有这个机制。

## 已实现的功能

### xkbcommon 键盘解码
- 接收 compositor 发送的 keymap fd → 创建 `xkb::Keymap` + `xkb::State`
- 按键解码：`keycode → keysym → VirtualKey`，支持任意键盘布局
- 旧的手写映射 `xkb_to_vk_raw()` 保留为 fallback

### 修饰键追踪
- v2: 通过 `xkb_state.update_mask()` 同步 `mods_depressed/latched/locked/group`
- v1: 同上

### 上下文事件
- `surrounding_text` — 输入框上下文文字/光标位置
- `content_type` — 输入框类型（hint/purpose）

## 开发中遇到的困难

### 1. 每 seat 只有一个输入法（核心问题）
Wayland 协议规定 `zwp_input_method_v1/v2` 每个 seat 只能有一个活跃的输入法。
- 在 KDE 上：ibus-daemon 已经注册为输入法，qianyan-IME 的连接虽成功但永远收不到 `activate` 事件
- 在 Weston 上：`weston-keyboard` 内置虚拟键盘抢占 slot，外部 IM 同样收不到 `activate`
- **kill ibus 会导致整个键盘失效**：因为 Wayland compositor 的键盘输入链依赖活跃的输入法

### 2. v1/v2 协议差异大
- 两个版本的 API 完全不同，需要分别实现
- v2 没有按键透传机制（没有 `forward_key`），unhandled 按键只能通过 `commit_string` 输出文本，对非文本键（方向键、Escape 等）无能为力
- v1 有 `context.key()` 和 `context.modifiers()` 用于透传未处理事件

### 3. 按键编码复杂
- Wayland 发送的 keycode 是 xkb keycode（evdev scancode + 8），不能直接用 evdev 码表
- 必须解析 compositor 发送的 keymap fd 才能正确解码
- 需要 `libxkbcommon`（通过 `xkbcommon` crate）来处理

### 4. Weston 测试环境限制
- Weston 13 默认启动 `weston-keyboard` 虚拟键盘模块，抢占 IM slot
- 需用 `weston --socket=wayland-1 &` 并在 weston-keyboard 前启动 IM
- `weston-editor` 是唯一支持 text-input 协议的简单测试应用

### 5. KDE 集成方案
要真正在 KDE 上使用，有两个可行路径：
- **做成 ibus 插件**（推荐）：实现 ibus DBus 协议，注册为 ibus 引擎，用户通过 KDE 的 ibus 设置切换
- **替换 ibus**：停止 ibus 服务，让 KDE 直接使用 qianyan-IME。需要处理 KDE 的输入法发现机制

### 6. 调试困难
- Wayland 协议错误通常表现为 "Broken pipe"（连接断开），难以定位具体原因
- 用 `WAYLAND_DEBUG=1` 可查看 protocol dump，但输出量大
- 建议先在 Weston 上验证协议实现，再到目标 compositor 上调试集成

## 测试方法

```bash
# 方式1：evdev 后端（通用，不依赖 compositor）
cargo run --bin qianyan-ime -- --backend=evdev

# 方式2：Wayland 后端 + Weston 测试
weston --socket=wayland-1 &
WAYLAND_DISPLAY=wayland-1 cargo run --bin qianyan-ime -- --backend=wayland
WAYLAND_DISPLAY=wayland-1 weston-editor

# 方式3：Wayland 后端 + KDE（需先停止 ibus 并抢占 IM slot）
# 不推荐，会导致键盘失效
```

## 依赖

```toml
# platform-linux/Cargo.toml
wayland-client = "0.31"
wayland-protocols = { version = "0.32", features = ["unstable"] }   # v1 协议需要 unstable feature
wayland-protocols-misc = "0.3"                                       # v2 协议
xkbcommon = "0.8"                                                    # 键盘解码
```

## 下一步

1. 实现 ibus 插件协议（DBus），让 qianyan-IME 作为 ibus 引擎运行
2. 或实现 fcitx5 插件协议，通过 `fcitx5-wayland-launcher` 集成
3. 完善 v1 的 `context.key()` 透传逻辑（当 IM 不处理按键时转发给应用）
4. 处理 v2 的 `zwp_input_popup_surface_v2` 实现候选窗口定位
