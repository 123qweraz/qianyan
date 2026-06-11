# 待修复问题

> 已修复的问题已从本文件移除，仅保留尚未修复的条目。

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

### H3. 缓存惊群 (pipeline 构建)
- **文件**: `crates/engine/src/pipeline/engine.rs:399-449`
- **问题**: 多个请求同时 miss 时，都在锁外构建昂贵 pipeline。
- **方案**: 将 pipeline 构建移到写锁内，双检锁模式。

### H6. 线程泄漏 (profile 预热)
- **文件**: `crates/engine/src/context.rs:106-109`
- **问题**: 每个 profile 预热 `thread::spawn` 后从不 join。
- **方案**: 使用线程池或存储 JoinHandle 等待完成。

### H9. Wayland PassThrough 按键静默丢失
- **文件**: `crates/platform-linux/src/hosts/wayland_host.rs:449-467`
- **问题**: `utf8_text.is_empty()` + 无虚拟键盘时，非文本键被静默吞掉。
- **方案**: 使用 uinput 转发按键。

---

## 🟡 中 (MEDIUM)

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

### M16. Wayland 连接探测泄漏
- **文件**: `crates/platform-linux/src/runtime.rs:156-159`
- **问题**: 为探测协议版本创建额外 Wayland 连接后再丢弃。
- **方案**: 复用同一连接或延迟探测。

---

## 🟢 低 (LOW)

### L6. vkbd SPACE+Backspace 全局 hack
- **文件**: `crates/platform-linux/src/hosts/vkbd.rs:347-362`
- **问题**: 为 Firefox 做的 hack 对所有应用生效，破坏撤销历史。
- **方案**: 仅对 Firefox 窗口生效，或使用 Wayland 协议。

---

## 架构与测试问题

### A2. clippy.toml 禁止 unwrap 但代码中仍有大量使用
- **文件**: `clippy.toml` 配置了 `disallowed_methods` 禁止 `.unwrap()`
- 但 test 代码和部分生产代码中仍有使用（尤其是 trie.rs 的测试代码）。

---

## 统计

| 严重级别 | 总数 |
|---------|------|
| 🔴 严重 (CRITICAL) | 4 |
| 🟠 高 (HIGH) | 3 |
| 🟡 中 (MEDIUM) | 6 |
| 🟢 低 (LOW) | 1 |
| 架构/测试 | 1 |
| **合计** | **15** |
