# Qianyan IME — 全部已知问题

> 代码审查发现的所有问题记录在此，供后续逐步修复。已修复的标注 ✅。

---

## 🔴 严重 (CRITICAL)

### C1. unsafe Mmap 并发文件修改风险
- **文件**: `crates/engine/src/trie.rs:62`
- **问题**: `Mmap::map()` 映射 `trie.data`，编译器(`compiler.rs:371-379`)可能并发重写该文件。读取悬空内存导致 UB。
- **方案**: 使用文件锁，或 mmap 前检查文件修改时间。

### C2. TOCTOU 竞争导致用户词典数据丢失
- **文件**: `crates/engine/src/config_manager.rs:120-251`
- **问题**: 多处 `load()`+clone+`store()` 模式，并发写入时旧数据覆盖新数据。
- **方案**: 全部替换为 `ArcSwap::rcu()`。

### C3. Windows: use-after-free EditSession
- **文件**: `crates/platform-windows/src/text_service.rs:39-44`
- **问题**: `EditSession` 栈变量传给 `RequestEditSession` 未加 `TF_ES_SYNC`，COM 异步回调时对象已析构。
- **方案**: 加 `TF_ES_SYNC` 或使用堆分配 + 引用计数。

### C4. Windows: 编译失败的模块引用
- **文件**: `crates/platform-windows/src/tsf.rs:1-6`
- **问题**: `use crate::platform::traits::*` 和 `use crate::engine::Processor` 引用不存在的模块路径。
- **方案**: 修正模块引用路径。

---

## 🟠 高 (HIGH)

### H1. UUID 使用纳秒时钟生成 8 字符
- **文件**: `crates/ui/src/web.rs:2944-2954`
- **问题**: 32 位输出 + 低熵源，每秒内碰撞概率极高。
- **方案**: 使用 `uuid::Uuid::new_v4()` 或 `rand::thread_rng()`。

### H2. 路径穿越 (create_dict_handler)
- **文件**: `crates/ui/src/web.rs:527-546`
- **问题**: 未调用 `safe_join()`，可写 JSON 到任意路径。
- **方案**: 对 `name` 和 `group` 字段做路径校验。

### H3. 缓存惊群 (pipeline 构建)
- **文件**: `crates/engine/src/pipeline/engine.rs:399-449`
- **问题**: 多个请求同时 miss 时，都在锁外构建昂贵 pipeline。
- **方案**: 将 pipeline 构建移到写锁内，双检锁模式。

### H4. RwLock 污染后静默失败
- **文件**: `crates/engine/src/pipeline/engine.rs:436`
- **问题**: `write().ok()?` 在锁污染时返回 None，所有输入处理静默失败。
- **状态**: ✅ 已修复（代码中已无此模式）

### H5. 生产代码残留 println!
- **文件**: `crates/engine/src/schemes/chinese.rs:457`
- **问题**: 每次中文查词输出 `DEBUG s4 final=`，严重拖慢输入。
- **状态**: ✅ 已修复（代码中已无此 println!）

### H6. 线程泄漏 (profile 预热)
- **文件**: `crates/engine/src/context.rs:106-109`
- **问题**: 每个 profile 预热 `thread::spawn` 后从不 join。
- **方案**: 使用线程池或存储 JoinHandle 等待完成。

### H7. IPC 帧无大小上限
- **文件**: `crates/ui/src/ipc/transport.rs:21-22`
- **问题**: `vec![0u8; len]` 读取 `u32::MAX` 长度可 OOM。
- **方案**: 加最大消息长度限制 (如 16MB)。

### H8. Trie 重复加载 4 次
- **文件**: `crates/ui/src/web.rs:1745-1808`
- **问题**: 四个 `OnceLock` 各自独立加载完整 Trie。
- **状态**: ✅ 已修复

### H9. Wayland PassThrough 按键静默丢失
- **文件**: `crates/platform-linux/src/hosts/wayland_host.rs:449-467`
- **问题**: `utf8_text.is_empty()` + 无虚拟键盘时，非文本键被静默吞掉。
- **方案**: 使用 uinput 转发按键。

### H10. IBus address 文件覆盖
- **文件**: `crates/platform-linux/src/hosts/ibus_backend.rs:661-668`
- **问题**: 获取 bus name 失败后仍重写 address 文件，破坏会话 IBus 输入。
- **方案**: 只在成功获取 name 后写入。

### H11. Wayland v1 修饰键未传入引擎
- **文件**: `crates/platform-linux/src/hosts/wayland_host_v1.rs:287`
- **问题**: shift/ctrl/alt 硬编码为 false，所有快捷键失效。
- **方案**: 从 xkb 状态读取修饰键。

---

## 🟡 中 (MEDIUM)

### M1. 二进制 Trie 解析损坏时静默默认 0
- **文件**: `crates/engine/src/trie.rs:535-599`
- **问题**: `unwrap_or_default()` 掩盖文件损坏。
- **状态**: ✅ 已修复

### M2. search_abbreviation 忽略 limit 参数
- **文件**: `crates/engine/src/pipeline/translators.rs:229-230`
- **问题**: 无视调用者 limit，始终返回最多 3000 条。
- **状态**: ✅ 已修复

### M3. ngram 数据双写 + TOCTOU
- **文件**: `crates/engine/src/processor/learning.rs:72-83`
- **问题**: `update_mru()` 通过 rcu 写一次，`insert_ngram()` 又用 `load()+store()` 写一次。
- **状态**: ✅ 已修复 — insert_ngram 改用 rcu()。

### M4. 浮点数截断丢数据
- **文件**: `crates/engine/src/schemes/chinese.rs:253,279`
- **问题**: `(weight * 0.8) as u32` 对 weight=1 结果=0，低频用户词被过滤。
- **状态**: ✅ 已修复

### M5. xdotool 每次按键调用子进程
- **文件**: `crates/ui/src/slint_window.rs:127-142`
- **问题**: 每次候选更新都 spawn `xdotool`，快速打字时显著降低性能。
- **状态**: ✅ 已修复 — 使用 `OnceLock` 缓存屏幕尺寸。

### M6. session 页面导航死代码
- **文件**: `crates/engine/src/processor/commands.rs:8-25`
- **问题**: `NextPage`/`NextCandidate` 中无意义的双重调用。
- **状态**: ✅ 已修复 — 移除第二次调用。

### M7. TCP 监听线程泄漏
- **文件**: `src/main.rs:366-427`
- **问题**: 子进程启动失败时监听线程永远阻塞。
- **状态**: ✅ 已修复 — 添加 10s 读超时。

### M8. tray 事件线程 sleep(500ms)
- **文件**: `src/main.rs:421`
- **问题**: 打开配置中心时冻结 tray 菜单半秒。
- **状态**: ✅ 已修复 — 异步启动 web 子进程。

### M9. OpenConfig TOCTOU 双重启动
- **文件**: `src/main.rs:349-427`
- **问题**: 两次并发点击可同时通过 AtomicBool 检查。
- **状态**: ✅ 已修复 — 使用 compare_exchange。

### M10. Web 配置中心 session 泄漏
- **文件**: `crates/ui/src/web.rs:2619-2631`
- **问题**: session 仅在新 session 创建时清理，长期运行服务器内存持续增长。
- **方案**: 添加 tokio 定时器清理。

### M11. Windows pipe 线程永不退出
- **文件**: `crates/platform-windows/src/tsf.rs:119-133`
- **问题**: 无 AtomicBool 停止信号，DLL 无法卸载。
- **方案**: 添加退出标志。

### M12. LockServer 空操作
- **文件**: `crates/platform-windows/src/class_factory.rs:43-45`
- **问题**: DLL 可能在 COM 对象存活时被卸载。
- **方案**: 实现 LockServer 增减模块锁计数。

### M13. Windows 注册部分失败无回滚
- **文件**: `crates/platform-windows/src/registry.rs:55-97`
- **问题**: 中间步骤失败留下孤儿注册表项。
- **方案**: 每一步失败时回滚之前的操作。

### M14. evdev Meta 键导致修饰键卡死
- **文件**: `crates/platform-linux/src/hosts/evdev_host.rs:326-339`
- **问题**: 强制发送 Ctrl/Shift/Alt release 后实际物理键仍按下。
- **方案**: 跟踪物理键状态，只转发到 uinput。

### M15. 剪贴板初始化失败永久缓存
- **文件**: `crates/platform-linux/src/hosts/vkbd.rs:295-305`
- **问题**: `OnceLock<Option>` 一旦失败存储 None，中文输入永远失效。
- **状态**: ✅ 已修复 — 失败后可重试初始化。

### M16. Wayland 连接探测泄漏
- **文件**: `crates/platform-linux/src/runtime.rs:156-159`
- **问题**: 为探测协议版本创建额外 Wayland 连接后再丢弃。
- **方案**: 复用同一连接或延迟探测。

### M17. 配置锁污染回退到错误设备
- **文件**: `crates/platform-linux/src/runtime.rs:27-49`
- **问题**: 锁污染时静默回退到 `/dev/input/event4`。
- **方案**: 记录错误并让用户手动指定。

### M18. web-settings TCP 重连无上限
- **文件**: `crates/web-settings/src/main.rs:46-55`
- **问题**: TCP 连接失败后无限重试。
- **方案**: 设置最大重试次数。

### M19. web-settings TCP 写错误静默忽略
- **文件**: `crates/web-settings/src/main.rs:76-78`
- **问题**: 连接断开后事件静默丢失。
- **方案**: 检测写入失败并尝试重连。

---

## 🟢 低 (LOW)

### L1. sound.rs 音频线程不 join
- **文件**: `crates/engine/src/sound.rs:53`
- **问题**: panic 静默丢失。
- **状态**: ✅ 已修复 — 在 Drop 中 join 线程。

### L2. compiler.rs fs::copy 失败静默丢弃
- **文件**: `crates/engine/src/compiler.rs:371-379`
- **问题**: `let _ = fs::copy(...)` 静默丢失新编译结果。
- **状态**: ✅ 已修复 — 增加日志警告。

### L3. 配置文件损坏静默返回 None
- **文件**: `crates/core/src/config.rs:672-675`
- **问题**: 配置文件损坏时无日志，用户不知设置为何"被重置"。
- **状态**: ✅ 已修复 — 增加 log::warn!。

### L4. Wayland layer exit 死代码
- **文件**: `crates/ui/src/wayland_layer.rs:622,719`
- **问题**: `exit: AtomicBool` 只检查从未设置。
- **状态**: ✅ 已修复 — 移除死字段和检查。

### L5. Super 键错误映射到 Control
- **文件**: `crates/platform-linux/src/hosts/wayland_host.rs:735`
- **问题**: Super 键被当成 Control 处理。
- **状态**: ✅ 已修复 — 放行（返回 None，不拦截）。

### L6. vkbd SPACE+Backspace 全局 hack
- **文件**: `crates/platform-linux/src/hosts/vkbd.rs:347-362`
- **问题**: 为 Firefox 做的 hack 对所有应用生效，破坏撤销历史。
- **方案**: 仅对 Firefox 窗口生效，或使用 Wayland 协议。

### L7. `config_manager.rs:master_config_write` 总是 Ok
- **文件**: `crates/engine/src/config_manager.rs:70-71`
- **问题**: 返回类型 Result 但总是成功，caller 检查 `if let Ok` 恒真。
- **状态**: ✅ 已修复 — 改为返回 `&mut Config`。

### L8. actor.rs Exit 消息不退出循环
- **文件**: `crates/engine/src/processor/actor.rs:480`
- **问题**: `Exit => {}` 空匹配，循环继续阻塞。
- **状态**: ✅ 已修复 — run() 中直接 break。

### L9. compose.rs `|| true` 恒真条件
- **文件**: `crates/engine/src/pipeline/compose.rs:81`
- **问题**: `contains_key` 检查被绕过。
- **状态**: ✅ 已修复

### L10. cached_ngram_map 死代码
- **文件**: `crates/engine/src/pipeline/filters.rs:98,138-140`
- **问题**: 写入但从未读取。
- **状态**: ✅ 已修复 — 移除该字段。

### L11. dirty AtomicBool 死代码
- **文件**: `crates/engine/src/user_data.rs:46-47,191`
- **问题**: 脏标记实现不完整。
- **状态**: ✅ 已修复 — 移除该字段。

### L12. cursor_pos 死字段
- **文件**: `crates/engine/src/session.rs:35`
- **问题**: 恒为 0，从未使用。
- **状态**: ✅ 已修复 — 移除该字段。

### L13. tab_held_and_not_used 死字段
- **文件**: `crates/platform-linux/src/hosts/evdev_host.rs:96,245,308`
- **问题**: 从未设为 true，从未读取。
- **状态**: ✅ 已修复 — 移除该字段。

### L14. Segmentor 使用重复参数
- **文件**: `crates/engine/src/pipeline/segmentation.rs:6-14`
- **问题**: `base_syllables` 和 `single_syllables` 总是相同数据。
- **状态**: ✅ 已修复（trait 已只保留 base_syllables）

### L15. send_key tray 事件无处理
- **文件**: `src/main.rs:483`
- **问题**: Web UI 发送 send_key 命令但 tray 事件处理为空。
- **方案**: 实现或移除该功能。

---

## 架构与测试问题

### A1. 缺少 Rust 单元测试 ✅ (部分已修复)
- 当前全部使用外部 Python/Bash 脚本测试，无 `#[test]` 集成。
- 已在 `session.rs` 增加 6 个测试用例覆盖边界条件。
- 其他 crate 仍需要补充。

### A2. clippy.toml 禁止 unwrap 但代码中仍有大量使用
- **文件**: `clippy.toml` 配置了 `disallowed_methods` 禁止 `.unwrap()`
- 但 test 代码和部分生产代码中仍有使用（尤其是 trie.rs 的测试代码）。

### A3. web.rs 多处路径遍历风险
- `clear_user_dict` (line 1095): profile 未校验
- `delete_user_dict_entry` (line 1382): profile 未校验
- `import_user_data` (line 2722): profile 未校验
- **方案**: 统一使用 `safe_join()` 或正则校验 `[a-zA-Z0-9_-]+`。

---

## 统计

| 严重级别 | 总数 | 本次已修复 |
|---------|------|-----------|
| 严重 (CRITICAL) | 4 | 0 |
| 高 (HIGH) | 11 | 3 |
| 中 (MEDIUM) | 19 | 10 |
| 低 (LOW) | 15 | 13 |
| **合计** | **49** | **26** |
