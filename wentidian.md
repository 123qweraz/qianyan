1. 拼音编辑功能优化 (Vim-like Editing)

  问题分析：
  目前拼音缓冲区（buffer）的编辑仅支持末尾追加和回退（Backspace）。InputSession 虽然有 cursor_pos
  字段，但在键盘输入处理中未被充分利用。用户希望实现类似 Vim 的快捷键来快速移动光标、按音节跳转和删除。

  解决方案：
   1. 扩展 InputSession 状态：
       * 在 crates/engine/src/session.rs 的 InputSession 中增加 insert_mode: bool。
   2. 增强光标移动逻辑：
       * 在 InputSession 中实现 move_cursor_left(), move_cursor_right(), move_cursor_start(),
         move_cursor_end()。
       * 音节跳转 (w)： 利用 best_segmentation（已分词结果）计算下一个分词点的索引。
   3. 增强删除逻辑：
       * 实现 delete_word()：删除光标处的当前音节。
       * 实现 clear_buffer()：即 dd 功能。
   4. 快捷键绑定 (Tab 组合键)：
       * 修改 crates/engine/src/processor/mod.rs。当 tab_down 为 true 时，拦截 H/L/A/E/W/D 等键。
       * Tab + N/M → 左右移动光标。
       * Tab + A/E → 行首/行尾。
       * W → 按音节向后跳转。
       * DD → 清空缓冲区。
       * I / R → 切换插入/替换模式。
      tab +s 手动切换模糊音， 比如 输入 shun 按下快捷键就变成 sun 再按一下， 由变成shun ch c zh z 同理， 
   ---





qianyan-ime-gui内存随着打字缓慢升高的现象，话说为啥会渐渐升高，UI不是只负责展示文字，为啥会升高内存，是缓存了什么东西在gui吗
仅限wayland环境，在x11，不会，x11的内存稳定在10m以下

中/英文切换通知 没有

剪切板延时时间和backspace延时时间给用户开放，让用户可以在系统设置里改

组句算法没有写在拼音设置里

新词发现器完善

网页增加简体与繁体互转功能

系统设置 输入法方案  应该放在系统词典，同时优化下UI与设置，这些设置有问题，和托盘的词典方案没有联动


系统词典 搜索 增加 笔画 和 笔画辅助码 搜索 ，同时 搜索可以同时 搜多本词典，比如搜所有的字

生僻字

增加网页输入法


ai修bug加功能应该改一个git保存一下
要通过测试，cargo test
不懂要问


wayland下，这个输入法，会启动很多wl-copy


核心诊断报告

  1. 输入延迟 (Input Latency)
   * 问题：在 vkbd.rs 中，每次粘贴文本都有多次 thread::sleep，累积延迟高达
     80ms。这在快速打字时会感到明显的滞后感。
   * 缺失信号：uinput 发送按键后缺失了 SYN_REPORT 事件。这会导致 Linux
     内核可能产生额外的处理延迟，因为它在等待批处理结束。

  2. Wayland 进程/内存泄漏 (确诊)
   * 根源：wl-copy 的机制是启动后在后台运行以提供剪贴板内容。之前代码频繁启动 wl-copy
     但从未销毁，导致系统里残留大量僵尸进程，这正是你看到的“内存升高”和“进程多”的主因。
   * 修复状态：虽然我们刚刚加了 arboard 缓存，但还需要更彻底地处理 wl-copy
     的生命周期，或者确保它只在必要时启动。

  3. 字典查找瓶颈 (Dictionary Bottlenecks)
   * 低效搜索：search_wildcard（通配符搜索，如辅助码过滤）目前是 $O(N)$ 复杂度，即每次都要扫描整个 FST
     字典文件。
   * 重复建索引：在 web.rs 的“新词发现”接口中，每次请求都会重新扫描所有字典文件来构建一个 known_words
     集合。对于 10MB 的字典，这会消耗巨大的 CPU 和内存。

  4. 同步阻塞 (Sync Blocking)
   * 主线程阻塞：在 evdev_host.rs
     中，处理空格、回车等“同步键”时，主事件循环会同步等待后台搜索结果返回。如果字典查找变慢，你的键盘输入就会
     直接被卡死。
     架构问题
     1. 全局 Arc<Mutex<Processor>> 锁竞争 （严重）
     Processor 被作为单一全局互斥锁管理，在以下线程间共享：
     - src/main.rs:148 — 主线程创建
     - evdev_host.rs:90 — evdev 键盘处理循环
     - evdev_host.rs:207-314 — 后台检索线程（每次都 lock/unlock 多次）
     - wayland_host.rs:186 — Wayland 事件分发
     - main.rs:188-382 — 托盘事件处理
     - main.rs:439-477 — IPC 转发线程
     问题：每个按键事件触发多次 lock/unlock（后台检索线程在 evdev_host.rs:233、244、266、297、302、308 处反复加解锁）。高频输入时锁争用剧烈，尤其当检索或 GUI IPC 传输耗时较长时。
     建议：采用 Actor 模式，将 Processor 放在独立线程中通过 channel 通信，消除全局锁。
     2. 后台检索线程的复杂同步 （严重）
     evdev_host.rs:216-314 的后台检索线程使用 Condvar + Mutex<bool> 做同步，但设计中存在多个问题：
     - evdev_host.rs:228：循环 try_recv() 清空 channel（丢失事件）
     - evdev_host.rs:479-483：同步键（Space/Enter 等）等待 Condvar，但spurious wakeup 处理不完善
     - evdev_host.rs:218-226：PendingGuard 的 Drop 会无差别通知所有等待者
     建议：用 oneshot channel 或 tokio::sync::Notify 替代 condvar 模式。
     3. 用户数据写入时全量克隆 （高影响）
     config_manager.rs:99-107、112-120、124-132 的 insert_learned/insert_usage/insert_ngram 方法每次都：
     1. 克隆整个 HashMap<String, HashMap<String, Vec<(String, u32)>>>（深拷贝）
     2. 修改后通过 ArcSwap::store() 替换
     随使用时间增长，用户词典数据量累积，此 O(n) 克隆操作会越来越慢。
     建议：改用 dashmap 分片，或基于 ArcSwap 做 copy-on-write 时只 clone 必要分支。
     并发/线程安全
     4. 标准 Mutex 在 Wayland 事件队列中使用 （高影响）
     wayland_host.rs:186：
     let mut guard = match state.processor.lock() {
     Wayland 事件队列是单线程驱动的，但若 Processor 被其他线程长时间持有，会阻塞 Wayland 事件循环，导致 UI 冻结（尤其在 dispatch_pending + sleep(4ms) 模式 wayland_host.rs:435 下）。
     5. IPC 转发中的死锁风险 （中影响）
     main.rs:439-477 的 IPC 线程从 gui_rx 接收事件，通过 Unix socket 发送给 GUI 进程。同一时刻托盘线程持有 processor.lock() 再发送 GuiEvent。如果 IPC channel 满或 GUI 处理慢，可能形成锁依赖链。
     6. 未处理的 channel 关闭
     大量 let _ = tx.send(...) 忽略 SendError（例如 evdev_host.rs:505、main.rs:228）。当 GUI 进程崩溃时，这些错误无声消失，无法触发恢复逻辑。
     性能问题
     7. 拼音切分缓存未限制 （中影响）
     engine.rs:23：
     segment_cache: std::sync::RwLock<std::collections::HashMap<String, Vec<String>>>,
     缓存上限 100 条 engine.rs:69，但没有过期或 LRU 淘汰策略。高频输入变体（如 "nihao"、"ni"、"n"、"nih"）可能导致频繁缓存颠簸。
     8. Pipeline 缓存的读写锁争用 （中影响）
     engine.rs:352-366：每次 pipeline 查找先读锁查缓存，再写锁更新访问顺序。高频搜索时（每按键一次）读写锁在多个线程间争用。
     9. 模糊音变体生成每次完全重新计算 （中影响）
     translators.rs:197-211：每次按键都重新调用 fuzzy_variants_per_segment（segmentation.rs:182），用 BFS 生成所有变体并 HashSet::insert + sort。对多音节长输入，变体数量指数增长。
     10. IPC JSON 序列化开销 （中影响）
     main.rs:497-509 和 transport.rs:112：每个按键事件将整个候选列表通过 serde_json 序列化/反序列化。Unix socket IPC 本身很快，但 JSON 转换大量数据（候选词列表每次数百条）带来明显 CPU 开销。
     建议：改为 bincode 或 messagepack，或只传输当前页的子集（已在 update_gui_internal 中做了分页但 IPC 序列化的 GuiEvent::Update 未限制大小）。
     11. 全 FST 遍历的通配符搜索 （低影响）
     trie.rs:210 的 search_wildcard 和 trie.rs:143 的 build_word_index 都会遍历整个 FST。build_word_index 是惰性的，首次调用（如 has_word_in_dict）可能阻塞数秒。
     12. 调试日志在热路径中
     engine.rs:168-173 每次搜索都 log::info!，trie.rs:80 和 trie.rs:166 用 log::debug!。如果启用了 info/debug 日志级别，会对 I/O 造成明显影响。
     代码质量与工程问题
     13. unsafe 使用不当
     - wayland_host.rs:473、478、483：多处 unsafe { std::mem::transmute<u32, VirtualKey> }，依赖 VirtualKey 的内存布局，但无编译期断言。
     - trie.rs:54：unsafe { Mmap::map(&file)? } — 需要 unsafe 的理由未注释说明。
     - main.rs:42-66：Windows CreateMutexW 返回的 handle 在程序退出时未关闭（handle leak）。
     14. Processor 单一职责违反 （700+ 行）
     processor/mod.rs 包含：热键处理、FSM 状态机、候选导航、模糊音激活、按键批处理、全局过滤器等。EngineContext 也是包含 18 个字段的 "上帝结构体"。
     15. 声音播放依赖外部进程
     evdev_host.rs:669-680、wayland_host.rs:348-361：每次 Alert 都 spawn 一个 canberra-gtk-play 进程，额外 fork 开销大，且在无音频环境会静默失败。
     16. KEY_BATCH_DELAY_MS = 0 失效功能
     processor/mod.rs:22：
     const KEY_BATCH_DELAY_MS: u64 = 0;
     processor/mod.rs:321：elapsed < Duration::from_millis(0) 永远为 false，按键批处理功能实质禁用。但相关的大量代码（pending_key_buffer、process_batched_keys 等）保留未清理。
     17. lookup_tx channel 无界
     evdev_host.rs:203：std::sync::mpsc::channel::<()>() 无容量限制。在极端情况下（处理器卡住），后台检索线程队列可能无限增长。
