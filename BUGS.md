# Qianyan IME Bug 列表

> 生成日期: 2026-05-18
> 状态: 已修复 #1 (UB 指针转换), #2 (trigger_prefetch 线程 panic)

---

## 严重 Bug

### 1. ~~未定义行为：不安全的指针转换~~ ✅ 已修复
- **文件**: `src/engine/pipeline.rs:674-686`, `crates/engine/src/pipeline.rs:671-683`
- **问题**: `get_trie_from_pipeline` 将 `*const dyn Translator` fat pointer 直接转成 `*const TableTranslator` thin pointer，这是 UB。
- **修复**: 添加 `as_any()` 方法到 `Translator` trait，使用 `Any::downcast_ref()` 安全转换。

### 2. ~~线程泄漏：`trigger_prefetch` 无限制生成线程~~ ✅ 已修复
- **文件**: `src/engine/processor/mod.rs:485-522`
- **问题**: 每次击键都 `std::thread::spawn()` 新线程做预取，快速打字时会生成大量线程。已有 `AtomicBool` 限制并发为 1，但如果线程 panic 则 flag 永不清除，所有后续预取被阻塞。
- **修复**: 用 `catch_unwind` 包裹线程体，保证 panic 后仍重置 flag。

### 3. 状态机 Bug：`Consume` 效果错误调用 `handle_composing`
- **文件**: `crates/engine/src/processor/mod.rs:338-345`
- **问题**: `FsmEffect::Consume` 和 `FsmEffect::UpdateLookup` 调用了完全相同的 `handle_composing` 函数。Consume 应该直接消费事件，不应该触发 lookup 或 composing 逻辑。
- **影响**: 非编码按键在 Composing 状态下也会走完整 composing 流程，可能意外添加字符或触发 lookup。

### 4. 缓存逻辑错误：`TableTranslator` 缓存过滤逻辑错误
- **文件**: `crates/engine/src/pipeline.rs:156-161`
- **问题**: filter 逻辑 `word.starts_with(&query) || word.starts_with(last_q)` 中 `word.starts_with(last_q)` 是错误的，应该只保留匹配新 query 的候选。
- **影响**: 缓存命中时返回不相关的候选词。

### 5. 竞争条件：LRU 驱逐使用两把锁
- **文件**: `crates/engine/src/pipeline.rs:685-756`
- **问题**: `access_order` 和 `pipelines` 分别加锁，在两次加锁之间另一个线程可能已驱逐同一 pipeline 或添加新 pipeline，导致驱逐决策过时。
- **建议**: 用一把锁同时保护两个 map，或使用 `parking_lot::RwLock` 的 upgradable read guard。

---

## 高优先级新增

### 24. IBus `transmute<u32, VirtualKey>` 未定义行为
- **文件**: `crates/platform-linux/src/hosts/ibus_host.rs:33`
- **问题**: 任意 `u32` transmute 为 Rust 枚举是 UB。且 `keyval - 0x61` 对小写字母有效，但 IBus 传递大写 keysym (`0x41`)，`0x41 - 0x61` 下溢出产生垃圾值。
- **建议**: 用安全的 `match` 映射有效按键。

### 25. FSM Release 状态未处理
- **文件**: `crates/engine/src/processor/fsm.rs:117-135`
- **问题**: FSM 在 `CandidatesShown`/`Composing` 状态时，按键释放事件返回 `None`，然后被 `Action::Consume` 吞掉。修饰键（Shift/Ctrl）释放应该 PassThrough。
- **建议**: Release 事件对修饰键返回 `PassThrough`。

### 26. IBus CommitText 从未发送
- **文件**: `crates/platform-linux/src/hosts/ibus_host.rs:81-120`
- **问题**: `process_key_event` 返回 `true` 但从未发送 IBus `CommitText` D-Bus 信号或更新 preedit。
- **建议**: 用 `zbus` 发送 CommitText/UpdatePreedit 信号。

### 27. `SoundManager` drop `OutputStream` 导致声音播放无声
- **文件**: `crates/engine/src/sound.rs:44`
- **问题**: `drop(stream)` 后 `OutputStreamHandle` 无法播放（按 rodio 文档要求）。
- **建议**: 把 `OutputStream` 存到 `SoundManager` 结构体中。

### 28. Alt+字母组合总是 PassThrough
- **文件**: `crates/engine/src/processor/mod.rs:220-224`
- **问题**: 所有 `Alt`/`Ctrl`+字母都被 PassThrough，阻挡了 Alt 选字等 IME 功能。
- **建议**: 只放行已知系统快捷键。

### 29. 英/日方案始终收到空 `tries` HashMap
- **文件**: `crates/engine/src/processor/handlers.rs:147`
- **问题**: `SchemeContext.tries` 始终为空，`EnglishScheme::lookup()` 取 `tries.get("english")` 永远返回 `None`。
- **建议**: 从 pipeline 加载的字典填充 `tries`。

---

## 中等 Bug

### 6. CapsLock 事件被无条件吞掉
- **文件**: `crates/engine/src/processor/mod.rs:626-640`
- **问题**: 中文模式下 CapsLock 总是返回 `Consume`，即使 buffer 为空且 `capslock_pending` 未设置时也会吞掉事件。
- **影响**: CapsLock LED 状态可能不正确，用户无法正常切换大小写锁定。

### 7. switch_mode 状态泄漏
- **文件**: `crates/engine/src/processor/intents.rs:250-255`
- **问题**: 当 `switch_mode` 为 true 时，如果按下的按键不是已知的 profile 切换键，代码 fallthrough 到 `_ => {}` 然后返回 `Consume`，但没有重置 `switch_mode = false`。
- **影响**: 用户卡在 switch 模式，后续所有按键都被吞掉。

### 8. Backspace 清空 buffer 时返回 PassThrough
- **文件**: `crates/engine/src/processor/handlers.rs:288-290`
- **问题**: 当 backspace 在 composing 状态下清空 buffer 时，返回 `Action::PassThrough`，导致 backspace 事件传递给应用。
- **影响**: 应用会多删除一个用户不想删除的字符。

### 9. `get_preedit` 用字节索引切 UTF-8 字符串
- **文件**: `crates/engine/src/compositor.rs:39`
- **问题**: `ctx.session.buffer[current_pos..]` 中 `current_pos` 是字符计数，但切片使用字节索引。如果 buffer 包含多字节 UTF-8 字符会 panic。
- **影响**: 在某些边缘情况下（如用户直接输入 Unicode 字符）会崩溃。

### 10. `process_batched_keys` 忽略 `inject_char_internal` 的 None 返回值
- **文件**: `crates/engine/src/processor/mod.rs:284-292`
- **问题**: 当 `inject_char_internal` 返回 `None`（lookup 无结果）时，循环继续并最终返回 `Consume`，但实际没有任何字符被处理。
- **影响**: 可能吞掉应该透传给应用的按键。

### 11. `next_profile` 在 profiles 为空时返回空字符串
- **文件**: `crates/engine/src/processor/mod.rs:140-141`
- **问题**: 当 `enabled.is_empty()` 时返回空字符串，托盘显示空白。
- **建议**: 回退到默认 profile 名称。

### 12. 后台线程操作过期状态
- **文件**: `crates/platform-linux/src/hosts/evdev_host.rs:183-204`
- **问题**: 后台 lookup 线程获取 `p_bg.lock()` 后调用 `lookup()` 和 `update_phantom_action()`。但主线程可能已修改 processor 状态（如清空 buffer），后台线程操作的是过期状态。
- **影响**: 可能发出错误的 delete/insert 动作。

---

## 轻微 Bug / 设计问题

### 13. `check_auto_commit` 的 `candidates[1]` 访问脆弱
- **文件**: `crates/engine/src/processor/mod.rs:565-566`
- **问题**: `candidates[1]` 依赖短路求值保护（先检查 `len() == 1`）。如果 `len() == 0` 会 panic。虽然有 `!is_empty()` 保护，但很脆弱。
- **建议**: 改用 `candidates.get(1)` 或 `match_level` 检查。

### 14. `handle_idle` 在 `key_to_char` 返回 None 时静默丢弃按键
- **文件**: `crates/engine/src/processor/handlers.rs:19-56`
- **问题**: 如果 `is_letter(key)` 为 true 但 `key_to_char()` 返回 None，按键被静默丢弃，无任何反馈。

### 15. Shift 释放时意外触发 global filter
- **文件**: `crates/engine/src/processor/intents.rs:17-25`
- **问题**: 如果 buffer 仍有内容（来自之前的部分输入），释放 Shift 会意外启动 global filter。`shift_used_as_modifier` 只在 Shift 按下时重置。

### 16. Alert 动作错误处理缺失
- **文件**: `crates/platform-linux/src/hosts/evdev_host.rs:498-510`
- **问题**: `Alert` 动作 spawn `canberra-gtk-play`，如果该二进制不存在则静默失败。
- **建议**: 添加 fallback 或至少记录 warning。

### 17. 缓存 TTL 50ms 太短
- **文件**: `crates/engine/src/pipeline.rs:153`
- **问题**: 缓存 TTL 仅 50ms，正常打字速度下两次击键间隔通常超过 50ms，缓存几乎无效。
- **建议**: 提高到 200-500ms，或改用击键计数而非时间。

### 18. AntiTypo Smart 模式首次拦截无提示
- **文件**: `crates/engine/src/compositor.rs:217-229`
- **问题**: 第一次输入字典中不存在的词时被静默拦截，用户不知道为什么输入被拒绝。需要输入第二次才能通过。
- **建议**: 添加视觉或声音反馈告知用户输入被拦截。

### 19. `inject_text` 中 `check_auto_commit` 是死代码
- **文件**: `crates/engine/src/processor/mod.rs:23-39`
- **问题**: 如果 `lookup()` 返回 None，`has_dict_match` 为 false，`check_auto_commit` 必然返回 None。

### 20. `TableTranslator::translate` 缓存检查存在竞争
- **文件**: `crates/engine/src/pipeline.rs:148-168`
- **问题**: `last_query` 和 `cached_candidates` 分别加锁读取。在两次加锁之间另一个线程可能更新缓存，导致读到不一致的状态。
- **建议**: 用一个锁同时保护两个字段，或使用原子操作。

### 21. WaylandHost 是空壳
- **文件**: `crates/platform-linux/src/hosts/wayland.rs`
- **问题**: `WaylandHost` 只发现了 input-method 协议但没有绑定，不处理按键事件，不与 processor 通信。`run()` 只是无限循环调用 `blocking_dispatch`。
- **影响**: Wayland 模式完全不可用。

### 22. `process_modifiers` 对修饰键释放返回 PassThrough
- **文件**: `crates/engine/src/processor/intents.rs:29-39`
- **问题**: 修饰键（Ctrl/Alt/Shift/CapsLock）释放时，如果 buffer 非空，返回 `Consume`。但之前对释放事件本身返回 `PassThrough`，导致主机会发出多余的 key-up 事件。

### 23. `process_modifiers` 中 Shift 释放的 global filter 逻辑有误
- **文件**: `crates/engine/src/processor/intents.rs:17-25`
- **问题**: 如果候选词已提交（buffer 清空）后释放 Shift，`!ctx.session.buffer.is_empty()` 检查阻止了 filter。但如果 buffer 仍有之前残留内容，释放 Shift 会意外触发 filter。

### 30. `ConfigManager::new()` 用 `println!`
- **文件**: `crates/engine/src/config_manager.rs:37`
- **问题**: 作为后台守护进程时 stdout 可能不存在，导致 panic。
- **建议**: 改用 `log::info!`。

### 31. WebServer 端点多处 `todo!()`
- **文件**: `crates/ui/src/web.rs:150-188`
- **问题**: 路由处理函数用了 `todo!()`，访问即 panic。
- **建议**: 返回 `StatusCode::NOT_IMPLEMENTED`。

### 32. `get_config` 从磁盘直接读取而非用服务端状态
- **文件**: `crates/ui/src/web.rs:190-195`
- **问题**: 每次 GET 请求创建新 `Config::load()`，UI 做的修改不会反映在 GET 响应中。
- **建议**: 使用 `Arc<RwLock<Config>>` 服务端状态。

### 33. 多处 `#[allow(dead_code)]` 掩盖死代码
- **说明**: 大量未使用的函数/变体被允许静默存在，应审计并清理或实现。

### 34. `clippy.toml` 禁止 `unwrap` 但代码中大量使用
- **说明**: `disallowed-methods` lint 会在 CI 报错。

### 35. 多个函数超过 `cognitive-complexity-threshold = 30`
- **说明**: CI 可能因此失败，需重构或提高阈值。

### 36. 集成测试断言为空或过于简单
- **说明**: 测试实际不验证行为。

### 37. `deny.toml` 禁止 GPL 但可能传递依赖 GPL crate
- **说明**: `cargo deny check` 可能失败。
