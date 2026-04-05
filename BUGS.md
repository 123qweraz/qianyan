# Shian IME Bug 列表

> 生成日期: 2026-04-05
> 状态: 已修复 #1 (UB 指针转换)

---

## 严重 Bug

### 1. ~~未定义行为：不安全的指针转换~~ ✅ 已修复
- **文件**: `src/engine/pipeline.rs:674-686`, `crates/engine/src/pipeline.rs:671-683`
- **问题**: `get_trie_from_pipeline` 将 `*const dyn Translator` fat pointer 直接转成 `*const TableTranslator` thin pointer，这是 UB。
- **修复**: 添加 `as_any()` 方法到 `Translator` trait，使用 `Any::downcast_ref()` 安全转换。

### 2. 线程泄漏：`trigger_prefetch` 无限制生成线程
- **文件**: `src/engine/processor/mod.rs:485-522`
- **问题**: 每次击键都 `std::thread::spawn()` 新线程做预取，快速打字时会生成大量线程，没有线程池或信号量限制。
- **影响**: 线程耗尽、锁竞争、性能下降。
- **建议**: 使用线程池（如 `rayon`）或 semaphore 限制并发数。

### 3. 状态机 Bug：`Consume` 效果错误调用 `handle_composing`
- **文件**: `src/engine/processor/mod.rs:342-349`, `crates/engine/src/processor/mod.rs:338-345`
- **问题**: `FsmEffect::Consume` 和 `FsmEffect::UpdateLookup` 调用了完全相同的 `handle_composing` 函数。Consume 应该直接消费事件，不应该触发 lookup 或 composing 逻辑。
- **影响**: 非编码按键在 Composing 状态下也会走完整 composing 流程，可能意外添加字符或触发 lookup。

### 4. 缓存逻辑错误：`TableTranslator` 缓存过滤逻辑错误
- **文件**: `src/engine/pipeline.rs:159-164`, `crates/engine/src/pipeline.rs:156-161`
- **问题**: filter 逻辑 `word.starts_with(&query) || word.starts_with(last_q)` 中 `word.starts_with(last_q)` 是错误的，应该只保留匹配新 query 的候选。
- **影响**: 缓存命中时返回不相关的候选词。

### 5. 竞争条件：LRU 驱逐使用两把锁
- **文件**: `src/engine/pipeline.rs:688-762`, `crates/engine/src/pipeline.rs:685-756`
- **问题**: `access_order` 和 `pipelines` 分别加锁，在两次加锁之间另一个线程可能已驱逐同一 pipeline 或添加新 pipeline，导致驱逐决策过时。
- **建议**: 用一把锁同时保护两个 map，或使用 `parking_lot::RwLock` 的 upgradable read guard。

---

## 中等 Bug

### 6. CapsLock 事件被无条件吞掉
- **文件**: `src/engine/processor/mod.rs:647-660`, `crates/engine/src/processor/mod.rs:626-640`
- **问题**: 中文模式下 CapsLock 总是返回 `Consume`，即使 buffer 为空且 `capslock_pending` 未设置时也会吞掉事件。
- **影响**: CapsLock LED 状态可能不正确，用户无法正常切换大小写锁定。

### 7. switch_mode 状态泄漏
- **文件**: `src/engine/processor/intents.rs:250-255`, `crates/engine/src/processor/intents.rs:250-255`
- **问题**: 当 `switch_mode` 为 true 时，如果按下的按键不是已知的 profile 切换键，代码 fallthrough 到 `_ => {}` 然后返回 `Consume`，但没有重置 `switch_mode = false`。
- **影响**: 用户卡在 switch 模式，后续所有按键都被吞掉。

### 8. Backspace 清空 buffer 时返回 PassThrough
- **文件**: `src/engine/processor/handlers.rs:288-290`, `crates/engine/src/processor/handlers.rs:288-290`
- **问题**: 当 backspace 在 composing 状态下清空 buffer 时，返回 `Action::PassThrough`，导致 backspace 事件传递给应用。
- **影响**: 应用会多删除一个用户不想删除的字符。

### 9. `get_preedit` 用字节索引切 UTF-8 字符串
- **文件**: `src/engine/compositor.rs:39`, `crates/engine/src/compositor.rs:39`
- **问题**: `ctx.session.buffer[current_pos..]` 中 `current_pos` 是字符计数，但切片使用字节索引。如果 buffer 包含多字节 UTF-8 字符会 panic。
- **影响**: 在某些边缘情况下（如用户直接输入 Unicode 字符）会崩溃。

### 10. `process_batched_keys` 忽略 `inject_char_internal` 的 None 返回值
- **文件**: `src/engine/processor/mod.rs:288-296`, `crates/engine/src/processor/mod.rs:284-292`
- **问题**: 当 `inject_char_internal` 返回 `None`（lookup 无结果）时，循环继续并最终返回 `Consume`，但实际没有任何字符被处理。
- **影响**: 可能吞掉应该透传给应用的按键。

### 11. `next_profile` 在 profiles 为空时返回空字符串
- **文件**: `src/engine/processor/mod.rs:140-141`, `crates/engine/src/processor/mod.rs:140-141`
- **问题**: 当 `enabled.is_empty()` 时返回空字符串，托盘显示空白。
- **建议**: 回退到默认 profile 名称。

### 12. 后台线程操作过期状态
- **文件**: `crates/platform-linux/src/hosts/evdev_host.rs:183-204`
- **问题**: 后台 lookup 线程获取 `p_bg.lock()` 后调用 `lookup()` 和 `update_phantom_action()`。但主线程可能已修改 processor 状态（如清空 buffer），后台线程操作的是过期状态。
- **影响**: 可能发出错误的 delete/insert 动作。

---

## 轻微 Bug / 设计问题

### 13. `check_auto_commit` 的 `candidates[1]` 访问脆弱
- **文件**: `src/engine/processor/mod.rs:570-571`, `crates/engine/src/processor/mod.rs:565-566`
- **问题**: `candidates[1]` 依赖短路求值保护（先检查 `len() == 1`）。如果 `len() == 0` 会 panic。虽然有 `!is_empty()` 保护，但很脆弱。
- **建议**: 改用 `candidates.get(1)` 或 `match_level` 检查。

### 14. `handle_idle` 在 `key_to_char` 返回 None 时静默丢弃按键
- **文件**: `src/engine/processor/handlers.rs:19-56`, `crates/engine/src/processor/handlers.rs:19-56`
- **问题**: 如果 `is_letter(key)` 为 true 但 `key_to_char()` 返回 None，按键被静默丢弃，无任何反馈。

### 15. Shift 释放时意外触发 global filter
- **文件**: `src/engine/processor/intents.rs:17-25`, `crates/engine/src/processor/intents.rs:17-25`
- **问题**: 如果 buffer 仍有内容（来自之前的部分输入），释放 Shift 会意外启动 global filter。`shift_used_as_modifier` 只在 Shift 按下时重置。

### 16. Alert 动作错误处理缺失
- **文件**: `crates/platform-linux/src/hosts/evdev_host.rs:498-510`
- **问题**: `Alert` 动作 spawn `canberra-gtk-play`，如果该二进制不存在则静默失败。
- **建议**: 添加 fallback 或至少记录 warning。

### 17. 缓存 TTL 50ms 太短
- **文件**: `src/engine/pipeline.rs:155`, `crates/engine/src/pipeline.rs:153`
- **问题**: 缓存 TTL 仅 50ms，正常打字速度下两次击键间隔通常超过 50ms，缓存几乎无效。
- **建议**: 提高到 200-500ms，或改用击键计数而非时间。

### 18. AntiTypo Smart 模式首次拦截无提示
- **文件**: `src/engine/compositor.rs:217-229`, `crates/engine/src/compositor.rs:217-229`
- **问题**: 第一次输入字典中不存在的词时被静默拦截，用户不知道为什么输入被拒绝。需要输入第二次才能通过。
- **建议**: 添加视觉或声音反馈告知用户输入被拦截。

### 19. `inject_text` 中 `check_auto_commit` 是死代码
- **文件**: `src/engine/processor/mod.rs:23-39`, `crates/engine/src/processor/mod.rs:23-39`
- **问题**: 如果 `lookup()` 返回 None，`has_dict_match` 为 false，`check_auto_commit` 必然返回 None。

### 20. `TableTranslator::translate` 缓存检查存在竞争
- **文件**: `src/engine/pipeline.rs:151-172`, `crates/engine/src/pipeline.rs:148-168`
- **问题**: `last_query` 和 `cached_candidates` 分别加锁读取。在两次加锁之间另一个线程可能更新缓存，导致读到不一致的状态。
- **建议**: 用一个锁同时保护两个字段，或使用原子操作。

### 21. WaylandHost 是空壳
- **文件**: `crates/platform-linux/src/hosts/wayland.rs`
- **问题**: `WaylandHost` 只发现了 input-method 协议但没有绑定，不处理按键事件，不与 processor 通信。`run()` 只是无限循环调用 `blocking_dispatch`。
- **影响**: Wayland 模式完全不可用。

### 22. `process_modifiers` 对修饰键释放返回 PassThrough
- **文件**: `src/engine/processor/intents.rs:29-39`, `crates/engine/src/processor/intents.rs:29-39`
- **问题**: 修饰键（Ctrl/Alt/Shift/CapsLock）释放时，如果 buffer 非空，返回 `Consume`。但之前对释放事件本身返回 `PassThrough`，导致主机会发出多余的 key-up 事件。

### 23. `process_modifiers` 中 Shift 释放的 global filter 逻辑有误
- **文件**: `src/engine/processor/intents.rs:17-25`
- **问题**: 如果候选词已提交（buffer 清空）后释放 Shift，`!ctx.session.buffer.is_empty()` 检查阻止了 filter。但如果 buffer 仍有之前残留内容，释放 Shift 会意外触发 filter。
