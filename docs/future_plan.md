# 千言输入法 — 未来优化计划

## 现状概览

### 已有功能
- 词库：~234K 拼音 key，FST trie 存储（14MB data + 1.7MB index）
- 用户词典：自动学习新词，MRU 排序
- 用法排序：频率 + 最近衰减加分
- 上下文联想：bigram 共现加分（P(w₂|w₁)，从用户输入学习）
- 组句（策略 3）：回溯分区 + syllable_freq 评分 + bigram 加分
- 精确/简拼/前缀/模糊/纠错五层策略
- 各策略加权评分：`精确=3 > 简拼=2 > 前缀=1 > 纠错=1`

### 与商业输入法（搜狗、谷歌拼音等）的关键差距

| 差距 | 说明 | 影响 |
|------|------|------|
| 词库量级 | 约 40-60 万词汇，商业输入法 >200 万 | 长尾词（人名/地名/术语）覆盖不足 |
| 词频权重 | trie 的 weight 字段大部分为 0 | 排序不准确，新加词无法合理排名 |
| 预训练 ngram | 无，只有用户使用中学习的 bigram | 组句质量差，无上下文预判 |
| 组句算法 | 回溯分区 + 每区间取最高 weight 词，无 beam search | 整句输入准确率低 |
| 上下文长度 | 只追溯上一个词（bigram） | 无法利用更长上下文（trigram 及以上） |
| 联想 | 无 next-word 预测模式 | 缺少商业输入法的"联想"候选 |

---

## 阶段一：扩展词库 + 词频权重（低工作量，中效果）

### 目标
- 增加词库覆盖量（目标 200 万+ 词汇）
- 为所有词汇编入真实词频权重

### 做法
1. 找开源中文词库并转换格式：
   - THUOCL（清华词库）
   - 搜狗细胞词库（非官方导出）
   - 汉语词频统计表
   - 其他开源中文词典
2. 转成 `dicts/chinese/words/*.json` 格式
3. 从语料库统计词频，填入 weight 字段
4. 运行 `cargo run -- compile` 重新编译 trie

### 注意事项
- 多音字需要处理（词条可重复出现，不同拼音对应不同位置）
- 权重值需要归一化或与现有 weight 范围兼容
- FST 可以处理任意规模数据，无性能瓶颈

### 预计效果
- **高**：长尾词匹配率大幅提升
- **中**：排序更准确（有真实词频后）
- **低**：组句质量改善有限（还需要 ngram）

---

## 阶段二：预训练 ngram 语言模型（中工作量，高效果）

### 目标
- 预训练一个 bigram 或 trigram 语言模型
- 集成到候选排序和组句评分中

### 方案选择

#### 方案 A：轻量级纯 Rust bigram（推荐起步）
```
训练流程：
  1. 获取开源中文语料（维基百科 dump、人民日报、CLUECorpus2020 等）
  2. 用 jieba/pkuseg 分词
  3. 统计 (wᵢ, wᵢ₊₁) 共现 → 计算 logP(wᵢ₊₁|wᵢ)
  4. 存储为 JSON 或 MessagePack 文件

集成方式：
  1. 启动时加载到 HashMap<String, Vec<(String, f32)>>
  2. post_process 中用 LM 分数替代/补充现有 ngram_bonus
  3. 组句评分中用 LM 替代 syllable_freq
  ```
- **复杂度**：低-中（纯 Rust，零外部依赖）
- **存储**：50M 词对约 200-500MB（未压缩 JSON），可选压缩
- **查询**：O(1) HashMap 查找

#### 方案 B：Stupid Backoff（Google 方案）
```
S(wᵢ | wᵢ₋₁) = count(wᵢ₋₁, wᵢ) / count(wᵢ₋₁)
S(wᵢ | wᵢ₋₂, wᵢ₋₁) = count(wᵢ₋₂, wᵢ₋₁, wᵢ) / count(wᵢ₋₂, wᵢ₋₁) × 0.4
```
- **不**需要平滑，简单退火
- 纯 Rust 实现
- 可以支持 trigram，比方案 A 更好

#### 方案 C：KenLM（C++ FFI）
- KenLM 是业界标准库，支持 backoff + 插值平滑
- 需要写 Rust FFI 绑定，跨平台编译复杂
- 质量最高，但维护成本高

### 预计效果
- **高**：候选排序更合理（"我了→解"加分，"我了一解"减分）
- **高**：组句质量明显提升
- **中**：需要足够大的训练语料才能发挥效果

---

## 阶段三：组句改进 + Beam Search（中-高工作量，很高效果）

### 目标
- 用 beam search 替代现有回溯分区组句算法
- 用 LM + 词频替代 syllable_freq 评分

### Beam Search 原理

```
输入拼音串（如 woxiangchifan）

从左到右扫描所有拼音位置：
  在每个位置尝试字典中匹配的所有词
  每条候选路径分数 = P(词|拼音) × P(词|前文)^λ
                    (词频权重)    (语言模型概率)

每步只保留分数最高的 B 个候选路径（beam width，通常 10-50）
拼音结束后，输出分数最高的 N 个整句
```

### 和现有组句的区别

| 现有回溯分区 | Beam Search |
|-------------|-------------|
| 先 Viterbi 切分拼音音节 | 不事先切分，搜索中自然决定 |
| 每个分区单独选最高 weight 词 | 每个位置考虑所有可能词 |
| 评分用 syllable_freq + 简单 bigram | 评分用词频 × LM 概率 |
| 回溯分区有限（100 种） | beam width 控制搜索空间 |
| 输出一种切分方案 | 输出 top-N 整句 |

### 依赖
- 完成阶段一（词频权重）和阶段二（ngram LM）
- 需要 P(词|拼音) = 词频归一化
- 需要 P(词|前文) = LM 查询

---

## 阶段四：联想预测（低-中工作量，中效果）

### 做法
1. 用户提交候选词后，用 LM 查询最可能的后续 N 个词
2. 在候选区额外显示"联想"候选
3. 用户选择后自动提交

### 依赖
- 阶段二（ngram LM）

---

## 总结路线图

```
阶段一 [低工作量，中效果]
  └─ 扩展词库 + 词频权重
      │
      ▼
阶段二 [中工作量，高效果]
  └─ 预训练 ngram LM（推荐方案 A 或 B）
      │
      ├─→ 阶段三 [中-高，很高效果] → Beam Search 组句
      │
      └─→ 阶段四 [低-中，中效果]   → 联想预测

---

## 阶段五：按键可视化浮层（低工作量，中效果）

### 目标
- 实现独立的按键显示浮层（类似 keystroke），按下的键浮在屏幕上
- 零额外权限（不走 `/dev/input`，通过已有输入法后端获取按键事件）
- 在所有后端（evdev / Wayland / IBus）下一致工作

### 架构总览

```
后端 (3 个)                 主进程 → UI 通道              UI 进程
┌────────────┐  Processor    ┌────────────┐  GuiEvent    ┌────────────────────┐
│ evdev_host │  handle_key   │  processor │  KeyEvent    │ KeystrokeOverlay   │
│ wayland    │  → Action     │  actor.rs  │ ──────────→  │  ├ Wayland: 独立    │
│ ibus       │               │            │              │  │  layer-surface   │
└────────────┘               │  (已有)    │              │  │  + Skia 直渲染    │
                             └────────────┘              │  └ X11: Slint 窗口  │
                                                         └────────────────────┘
```

按键事件不新增采集路径，直接复用各后端已解析好的 `VirtualKey` / keysym，通过已有 `GuiEvent` 通道送到 UI 进程。

### 与 keystroke 项目的对比

| 对比项 | standalone keystroke | 本方案 |
|--------|---------------------|--------|
| 数据源 | `/dev/input/event*`（需 input 组） | 输入法后端（零额外权限） |
| 权限 | input 组或 root | 无需，走的 Wayland/X11/IBus 协议 |
| 集成度 | 独立进程 | 复用已有的 GuiEvent 通道 + Slint/Skia 管线 |
| 键名映射 | 自己维护 keycode → name 表 | 复用已有的 `VirtualKey` + Display trait |
| 浮层实现 | GTK4 窗口 | Wayland layer-shell overlay / Slint 窗口 |

---

### 详细技术方案

#### 5.1 新增文件

##### 5.1.1 `crates/ui/src/keystroke.slint`

新建 Slint 组件，用于 X11 后备窗口和 Wayland offscreen 渲染：

```slint
export component KeystrokeWindow inherits Window {
    title: "QianyanIMEKeystroke";
    in property <[string]> keys;
    in property <[string]> modifiers;
    in property <bool> is_visible: false;

    no-frame: true;
    background: transparent;
    always-on-top: true;

    width: main_rect.preferred-width;
    height: main_rect.preferred-height;

    main_rect := Rectangle {
        visible: is_visible;
        background: rgba(0, 0, 0, 180);
        border-radius: 8px;

        VerticalLayout {
            padding: 8px 16px;
            spacing: 4px;
            alignment: center;

            if modifiers.length > 0 : HorizontalLayout {
                spacing: 4px;
                alignment: center;
                for mod in modifiers : Rectangle {
                    background: rgba(255, 255, 255, 30);
                    border-radius: 4px;
                    padding: 2px 6px;
                    Text { text: mod; color: #cccccc; font-size: 14px; }
                }
            }

            HorizontalLayout {
                spacing: 6px;
                alignment: center;
                for key in keys : Text {
                    text: key;
                    color: #ffffff;
                    font-size: 20px;
                    font-weight: 700;
                }
            }
        }
    }
}
```

##### 5.1.2 `crates/ui/src/keystroke_overlay.rs`

核心实现，约 250-350 行。包含两部分：

**A. Wayland 独立 layer-surface（主路径）**

```rust
// 伪代码架构
pub struct KeystrokeOverlay {
    // 按键状态
    held_keys: Vec<String>,
    held_mods: Vec<String>,
    last_key_time: Instant,
    timeout_ms: u64,

    // Wayland 渲染（仅 Linux + Wayland）
    wl_renderer: Option<WaylandKeystrokeRenderer>,

    // X11 后备（仅 Linux + X11 / XWayland）
    slint_window: Option<KeystrokeWindow>,
}

struct WaylandKeystrokeRenderer {
    conn: Connection,
    surface: wl_surface::WlSurface,
    layer: LayerSurface,
    pool: SlotPool,
    // Skia 离线渲染
    skia_surface: skia_safe::Surface,
    typeface: skia_safe::Typeface,
}
```

**Wayland 连接流程：**
```
1. Connection::connect_to_env()
2. registry_queue_init → 绑定 zwlr_layer_shell_v1, wl_shm, wl_compositor
3. compositor_state.create_surface() → wl_surface
4. layer_shell.create_layer_surface(Overlay, "qianyan-ime-keystroke")
5. layer.set_exclusive_zone(-1)        // 不占布局空间
6. layer.set_keyboard_interactivity(None) // 不抢键盘焦点
7. layer.set_anchor(Anchor::BOTTOM)    // 底部居中
8. layer.set_size(400, 60)
9. layer.commit()
```

**Skia 渲染流程：**
```
1. 收到按键事件 → 更新 held_keys / held_mods
2. 创建 Skia surface: 计算文字总宽度 → 确定窗口尺寸
3. 画背景：skia_safe::Paint 填充半透明黑色圆角矩形
4. 分别绘制修饰键（Ctrl/Shift/Alt 等，灰色底）和字母键（白色大号文字）
5. 渲染到 pixels buffer (BGRA8888)
6. 通过 wl_shm pool 创建 wl_buffer
7. buffer.attach_to(layer.wl_surface())
8. surface.damage_buffer() + layer.commit()
9. 启动超时定时器（1.5s 后无新按键 → 发送 HideKeystroke 到 Wayland 线程）
```

**线程模型：**
- Wayland 渲染在自己线程运行（与主 Wayland 显示器线程分离）
- 通过 `mpsc::channel` 接收 `KSCmd::Show { keys, mods }` / `KSCmd::Hide` / `KSCmd::Exit`
- 避免阻塞 UI 事件循环

**B. X11 后备 Slint 窗口**

当 `WAYLAND_DISPLAY` 未设置或 layer-shell 不可用时：
- 直接用 `KeystrokeWindow` Slint 组件
- 通过 `set_keys()` / `set_modifiers()` 更新
- 窗口始终显示，但内容透明（visible 控制显隐）
- 位置：底部居中，用 xdotool 获取屏幕尺寸计算

#### 5.2 修改文件

##### 5.2.1 `crates/ui/src/lib.rs`

```rust
#[derive(Debug, Clone)]
pub enum GuiEvent {
    // ... 已有变体 ...

    /// 按键事件（用于按键可视化浮层）
    KeyEvent {
        keys: Vec<String>,        // 当前按下的字母/符号键列表
        modifiers: Vec<String>,   // 当前按住的修饰键列表 (Ctrl, Shift, Alt, Super)
    },
}
```

##### 5.2.2 `crates/ui/src/gui_slint.rs`

在 `handle_event()` 和 `handle_ipc_event()` 中添加处理：

```rust
GuiEvent::KeyEvent { keys, modifiers } => {
    for d in displays.iter_mut() {
        if let Some(ko) = d.as_any().downcast_ref::<KeystrokeOverlay>() {
            ko.update_keys(&keys, &modifiers);
        }
    }
}
```

`KeystrokeOverlay` 实现 `CandidateDisplay` trait（或作为独立对象在 displays Vec 中）。

注意：displays Vec 中的 `KeystrokeOverlay` 与候选窗口并列，都需要 `Box<dyn CandidateDisplay>`。但因为 `CandidateDisplay` trait 没有按键相关方法，需要额外方式路由事件：

**方案 A：KeystrokeOverlay 不实现 CandidateDisplay，而是作为全局单例被 `handle_event` 直接调用。**

```rust
// gui_slint.rs
thread_local! {
    static KEYSTROKE: RefCell<Option<KeystrokeOverlay>> = const { RefCell::new(None) };
}

fn handle_event(...) {
    // ...
    GuiEvent::KeyEvent { keys, modifiers } => {
        KEYSTROKE.with(|k| {
            if let Some(ref mut ko) = *k.borrow_mut() {
                ko.update_keys(&keys, &modifiers);
            }
        });
    }
}
```

**方案 B：KeystrokeOverlay 放到 displays Vec 中，但 CandidateDisplay 加默认方法**

```rust
pub trait CandidateDisplay {
    // ... 现有方法 ...
    
    /// 更新按键显示（默认空实现，不破坏现有实现）
    fn update_keystroke(&mut self, _keys: &[String], _modifiers: &[String]) {}
}
```

推荐方案 B，更符合现有架构。

##### 5.2.3 各后端改动

**evdev_host.rs**（在按键循环中 ~300 行附近）：

```rust
// 在按键事件处理循环中，在 val == 1 / val == 0 分支里：
fn send_keystroke_event(gui_tx: &Option<Sender<GuiEvent>>, held_keys: &HashSet<Key>, vk: Option<VirtualKey>) {
    let Some(ref tx) = gui_tx else { return };
    
    // 收集当前按住的修饰键
    let mut mods = Vec::new();
    if held_keys.contains(&Key::KEY_LEFTSHIFT) || held_keys.contains(&Key::KEY_RIGHTSHIFT) { mods.push("Shift".into()); }
    if held_keys.contains(&Key::KEY_LEFTCTRL) || held_keys.contains(&Key::KEY_RIGHTCTRL) { mods.push("Ctrl".into()); }
    if held_keys.contains(&Key::KEY_LEFTALT) || held_keys.contains(&Key::KEY_RIGHTALT) { mods.push("Alt".into()); }
    if held_keys.contains(&Key::KEY_LEFTMETA) || held_keys.contains(&Key::KEY_RIGHTMETA) { mods.push("Super".into()); }
    
    // 收集所有当前按住的非修饰键 + 当前按下的键
    let mut keys: Vec<String> = held_keys.iter()
        .filter(|k| !matches!(k, Key::KEY_LEFTSHIFT | Key::KEY_RIGHTSHIFT | Key::KEY_LEFTCTRL
            | Key::KEY_RIGHTCTRL | Key::KEY_LEFTALT | Key::KEY_RIGHTALT
            | Key::KEY_LEFTMETA | Key::KEY_RIGHTMETA | Key::KEY_COMPOSE))
        .filter_map(|k| evdev_to_virtual(*k))
        .map(|vk| vk_display_name(vk))
        .collect();
    
    let _ = tx.send(GuiEvent::KeyEvent { keys, modifiers: mods });
}
```

**wayland_host.rs**（在 `Event::Key` handler 中 ~199 行）：

关键已通过 xkbcommon 解析为 `(VirtualKey, utf8_text)`，可直接用。

```rust
Event::Key { key, state, .. } => {
    // 已有处理...
    
    // 新增：发送按键可视化事件
    let mut held_mods = Vec::new();
    // 从 xkb_state 读取修饰键
    if let Some(ref xkb_st) = state.xkb_state {
        let depressed = xkb_st.serialized_mods().depressed;
        if depressed & 0x0001 != 0 { held_mods.push("Shift".into()); }  // MOD_SHIFT
        if depressed & 0x0004 != 0 { held_mods.push("Ctrl".into()); }   // MOD_CONTROL
        if depressed & 0x0008 != 0 { held_mods.push("Alt".into()); }    // MOD_ALT
        if depressed & 0x0040 != 0 { held_mods.push("Super".into()); }  // MOD_LOGO
    }
    // 收集当前 held 的非修饰键...
    let _ = state.gui_tx.send(GuiEvent::KeyEvent { keys, modifiers: held_mods });
}
```

**ibus_backend.rs**（在 `process_key_event` 中）：

```rust
// 从 keyval + state 提取修饰键和按键名
let mut mods = Vec::new();
if state & MOD_SHIFT != 0 { mods.push("Shift"); }
if state & MOD_CTRL != 0 { mods.push("Ctrl"); }
if state & MOD_ALT != 0 { mods.push("Alt"); }
let key_name = keysym_display_name(keyval);
let _ = gui_tx.send(GuiEvent::KeyEvent {
    keys: vec![key_name],
    modifiers: mods.into_iter().map(String::from).collect(),
});
```

#### 5.3 VirtualKey → 显示名称映射

在 `crates/engine/src/keys.rs` 中新增：

```rust
impl VirtualKey {
    /// 返回人类可读的按键名称（用于按键可视化浮层）
    pub fn display_name(self) -> &'static str {
        use VirtualKey::*;
        match self {
            A | B | C | D | E | F | G | H | I | J | K | L | M
            | N | O | P | Q | R | S | T | U | V | W | X | Y | Z => "A-Z",
            Digit0 => "0", Digit1 => "1", Digit2 => "2", Digit3 => "3",
            Digit4 => "4", Digit5 => "5", Digit6 => "6", Digit7 => "7",
            Digit8 => "8", Digit9 => "9",
            Space => "␣", Enter => "↵", Tab => "⇥", Backspace => "⌫",
            Esc => "⎋", CapsLock => "⇪",
            Shift => "⇧", Control => "Ctrl", Alt => "Alt",
            Left => "←", Right => "→", Up => "↑", Down => "↓",
            PageUp => "⇞", PageDown => "⇟",
            Home => "↖", End => "↘", Delete => "⌦",
            Grave => "`", Minus => "-", Equal => "=",
            LeftBrace => "[", RightBrace => "]", Backslash => "\\",
            Semicolon => ";", Apostrophe => "'",
            Comma => ",", Dot => ".", Slash => "/",
        }
    }
    
    /// 是否为修饰键
    pub fn is_modifier(self) -> bool {
        matches!(self, VirtualKey::Shift | VirtualKey::Control | VirtualKey::Alt)
    }
}
```

#### 5.4 按键超时消除

为避免按键释放事件丢失导致按键卡在屏幕上：

```
- 每次收到 press 事件 → 记录该键，刷新超时计时器
- 每次收到 release 事件 → 移除该键
- 如果 1.5 秒内没有任何按键事件 → 清空显示
- 超时在 Wayland 线程中处理（通过 timerfd 或循环中检查 elapsed）
```

Wayland 线程循环结构：

```rust
loop {
    // 1. 处理所有待处理命令
    while let Ok(cmd) = rx.try_recv() {
        match cmd { /* ... */ }
    }
    
    // 2. 检查超时
    if let Some(deadline) = hide_deadline {
        if Instant::now() >= deadline {
            send_hide_to_wayland_thread();
            hide_deadline = None;
        }
    }
    
    // 3. Wayland dispatch（带 timeout）
    // ...
}
```

#### 5.5 配置项

在 `Config` 中添加（`crates/core/src/config.rs`）：

```rust
pub struct LinuxConfig {
    // ... 已有字段 ...
    
    /// 是否启用按键可视化浮层
    #[serde(default = "default_keystroke_enabled")]
    pub keystroke_enabled: bool,
    
    /// 按键浮层位置（bottom-center / top-center）
    #[serde(default)]
    pub keystroke_position: String,  // "bottom-center" | "top-center"
    
    /// 按键浮层超时（毫秒）
    #[serde(default = "default_keystroke_timeout")]
    pub keystroke_timeout_ms: u64,
}

fn default_keystroke_enabled() -> bool { false }
fn default_keystroke_timeout() -> u64 { 1500 }
```

#### 5.6 IPC 支持

在主进程 ↔ GUI 子进程的 IPC 中添加 `MainToGui::KeyEvent` 变体：

```rust
// crates/ui/src/ipc/transport.rs
pub enum MainToGui {
    // ... 已有变体 ...
    KeyEvent {
        keys: Vec<String>,
        modifiers: Vec<String>,
    },
}
```

IPC 路径的 `handle_ipc_event()` 中同步处理。

#### 5.7 渲染细节

**用 Skia 直接绘制文字（不经过 Slint）：**

```rust
fn render_keystroke(&self, keys: &[String], mods: &[String]) -> (Vec<u8>, u32, u32) {
    // 1. 计算文字尺寸
    let font = skia_safe::Font::new(&self.typeface, 20.0);
    let mod_font = skia_safe::Font::new(&self.typeface, 14.0);
    
    let mut total_w = 40u32; // padding
    let mut total_h = 60u32;
    
    // 2. 创建 Skia surface
    let mut surface = skia_safe::Surface::new_raster_n32_premul(
        (total_w as i32, total_h as i32)
    ).expect("create skia surface");
    let canvas = surface.canvas();
    
    // 3. 绘制背景
    let mut bg = skia_safe::Paint::default();
    bg.set_color(skia_safe::Color::from_argb(180, 0, 0, 0));
    bg.set_anti_alias(true);
    canvas.draw_rrect(
        skia_safe::RRect::new_rect_radii(
            skia_safe::Rect::from_wh(total_w as f32, total_h as f32),
            &[8.0; 4],
        ),
        &bg,
    );
    
    // 4. 绘制修饰键（灰色小标签）
    let mut x = 20.0;
    for mod_name in mods {
        // 绘制小矩形背景
        let mut mod_bg = skia_safe::Paint::default();
        mod_bg.set_color(skia_safe::Color::from_argb(30, 255, 255, 255));
        // ...
        canvas.draw_string(mod_name, (x, 25.0), &mod_font, &mod_bg);
        x += mod_name.len() as f32 * 9.0 + 8.0;
    }
    
    // 5. 绘制按键文字（白色大号）
    let mut text_paint = skia_safe::Paint::default();
    text_paint.set_color(skia_safe::Color::WHITE);
    for key_name in keys {
        canvas.draw_string(key_name, (x, 42.0), &font, &text_paint);
        x += 24.0;
    }
    
    // 6. 读取像素
    let image = surface.image_snapshot();
    let pixmap = image.to_pixmap_downscaled().expect("to_pixmap");
    (pixmap.pixels.to_vec(), total_w, total_h)
}
```

实际实现中会使用 `skia_safe::Typeface` 和中文字体（与候选窗口字体一致），并在首次显示时创建 surface，之后复用。

#### 5.8 create_displays 改动

`crates/ui/src/gui_slint.rs` 的 `create_displays()` 中，当 `config.linux.keystroke_enabled` 为 true 时，追加一个 `KeystrokeOverlay` 到 displays：

```rust
fn create_displays(config: &Config) -> Vec<Box<dyn CandidateDisplay>> {
    let mut displays: Vec<Box<dyn CandidateDisplay>> = Vec::new();
    
    // 创建主候选窗口（现有逻辑）...
    
    // 创建按键可视化浮层
    if config.linux.keystroke_enabled {
        if let Some(ko) = KeystrokeOverlay::new(config) {
            log::debug!("[GUI_DEBUG] Using KeystrokeOverlay");
            displays.push(Box::new(ko));
        }
    }
    
    displays
}
```

### 工作量估算

| 任务 | 文件 | 预计代码行 | 难度 |
|------|------|-----------|------|
| 新建 keystroke.slint | `crates/ui/src/keystroke.slint` | ~60 | 低 |
| 新建 keystroke_overlay.rs | `crates/ui/src/keystroke_overlay.rs` | ~300 | 中 |
| 修改 lib.rs（GuiEvent + 模块声明） | `crates/ui/src/lib.rs` | ~10 | 低 |
| 修改 gui_slint.rs（路由 + create_displays） | `crates/ui/src/gui_slint.rs` | ~30 | 低 |
| 修改 evdev_host.rs（发按键事件） | `crates/platform-linux/src/hosts/evdev_host.rs` | ~30 | 低 |
| 修改 wayland_host.rs（发按键事件） | `crates/platform-linux/src/hosts/wayland_host.rs` | ~20 | 低 |
| 修改 ibus_backend.rs（发按键事件） | `crates/platform-linux/src/hosts/ibus_backend.rs` | ~20 | 低 |
| 修改 keys.rs（display_name 映射） | `crates/engine/src/keys.rs` | ~30 | 低 |
| 修改 config.rs（新配置项） | `crates/core/src/config.rs` | ~15 | 低 |
| 修改 ipc/transport.rs（IPC 支持） | `crates/ui/src/ipc/transport.rs` | ~5 | 低 |
| **总计** | | **~520** | |

### 与 standalone keystroke 项目的差异说明

| 方面 | standalone keystroke | 本项目方案 |
|------|-------------------|-----------|
| 事件源 | `/dev/input/event*`, evdev crate | 已有输入法后端 |
| 权限 | 需要 `input` 组 | 无需 |
| 键盘布局 | 自己查询 compositor IPC | 各后端已有 xkb 状态 |
| 浮层渲染 | GTK4 窗口 | Wayland layer-shell + Skia (Wayland) / Slint 窗口 (X11) |
| 配置 | 独立 TOML 文件 | 统一 Config 结构体 |
| 键名图标 | Nerd Font 图标 | Unicode 符号（← ↑ ↵ ⇧ ⌫ 等） |

### 实现路线图

```
第1步: keys.rs 加 display_name() + is_modifier()
第2步: config.rs 加 keystroke 配置项
第3步: lib.rs 加 GuiEvent::KeyEvent
第4步: transport.rs 加 IPC 变体
第5步: 创建 keystroke.slint（X11 备用）
第6步: 创建 keystroke_overlay.rs（Wayland + X11）
第7步: gui_slint.rs 集成
第8步: 三个后端发送 KeyEvent
第9步: 测试和微调
```

