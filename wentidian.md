

网页上，应该把系统词典和用户词典分开，两个设置， 系统词典，增加打开词典所在目录的功能

新词发现器，在网页增加新功能，用户导入txt文本，可以和系统词库对比，发现新词，支持将导出成txt格式或者字典的json格式，做成新字典，自动给发现的汉字加上拼音

编辑功能， tab ＋h ＋l，改成左右移动光标
增加更多的光标移动功能，tab＋a 移动到字母行首，tab＋e移动到字母行尾， w一个个音节移动， dd删掉整个拼音，dw删掉一个音节，
编辑功能，i插入，r替换，i是在光标插入字母，r是替换光标出字母，有点想backspace和delete的差别

slint bug
slint emoji不会显示成彩色的，而是黑白的
Emoji 颜色:
       * 这是由于 Slint 在某些 Linux 环境下默认不加载彩色字体库。需要显式配置字体回退路径或使用支持彩色 Emoji
         的渲染后端。
在wayland时，候选栏显示不全，在x11正常，但是我不清楚原因

设置优化

内置候选栏
在光标附近显示候选词窗口

桌面通知
通过系统通知显示候选词（可与内置候选栏同时开启）

显示候选窗口
控制拼写时是否出现选词框

[已完成] 这三个设置已经合并成"候选词UI显示模式"下拉框，四个选项（输入法候选栏/系统桌面通知/同时显示/无）

[已完成] 拼音算法开关已在页面完全开放（模糊拼音、前缀匹配、简拼、调频、自动造词、精准匹配自动上屏、自动组句句模式）

另外，现在的自动组句算法和造词算法混淆了，现在的自动组句算法组成的东西也会进入用户词典，显然这是不对，还有简拼的词组也进入到了用户，比如我用zm打出怎么，然后怎么就变成用户词，实际不应该进入词典，还有，自动组句算法也是，比如wowangjichongdianle，我忘记充电了，这是程序根据词典自动给出了，也不应该进入用户词典，只有自动造词算法，比如我前面打了 fuzhu 辅助 然后 打 ma 码 然后 下次就能 打出 辅助码 这才是一开始用户词典的设计

中英混输 优化， 还没想好

网页增加新功能， 数字转汉字大写金额，做个网页一个接受普通数字， 一个负责转换成大写金额， 然后提供复制功能
还有什么输入法喜欢做的小功能呢  日期与时间 ，也做网页上

[已完成] 中英文切换通知：LinuxConfig 新增 show_toggle_notification 字段，LinuxNotifyDisplay 通过独立开关控制，系统设置页增加对应复选框



存疑问题点

1. 频繁写操作下的 ArcSwap 性能瓶颈
  在 crates/engine/src/processor/learning.rs 的 update_mru 函数中：
   * 问题： 每次记录用户输入时，你都调用了 history.rcu(|hist| { let mut clone = (**hist).clone(); ... })。
   * 分析： ArcSwap 非常适合“读多写少”，但 UserDictData 是一个嵌套的
     HashMap。随着用户词库增大，每次记录词频都要全量克隆整个嵌套 Map。在高频输入或批量处理时，这会导致显著的 CPU
     开销和内存抖动。
   * 建议： 考虑将用户数据拆分为更细粒度的分片，或者在内存中使用更高效的读写锁结构，仅在持久化时才序列化。

  2. Linux IPC 机制的健壮性 (Main <-> GUI)
  在 src/main.rs 的 Linux 部分：
   * 问题 1：阻塞风险。 GuiEvent::HideAndAck 会阻塞事件转发线程，等待 GUI 的 Ack。如果 GUI
     因为重绘或系统负载高而响应缓慢（超过 100ms），会直接拖慢整个输入法主循环的响应速度。
   * 问题 2：崩溃恢复。 如果 qianyan-ime-gui 进程崩溃，主进程通过 stream_guard.take()
     标记连接关闭并跳出循环，但目前代码中没有看到自动重启 GUI
     进程的逻辑。用户会发现候选框突然消失且无法恢复，只能重启整个输入法。

  3. Pipeline 缓存策略过于简单
  在 crates/engine/src/pipeline/engine.rs 中：
   * 问题： segment_cache（分词缓存）的大小被硬编码为 100。
   * 分析： 缓存达到 100 后就不再更新。对于长期的输入环境，这几乎等同于没有缓存。
   * 建议： 实现一个简单的 LRU（最近最少使用）淘汰策略，或者根据内存占用动态调整缓存大小。

  4. FuzzyPinyinSearcher 的 DFS 深度风险
  在 crates/engine/src/pipeline/translators.rs 中：
   * 问题： FuzzyPinyinSearcher 使用递归 DFS 进行模糊音查找。
   * 分析： 虽然有 trie.has_prefix
     剪枝，但对于超长输入（如连写一长串拼音且包含大量模糊音选项时），递归深度和分叉数可能会呈指数级增长。
   * 建议： 增加一个最大搜索深度或最大分支尝试次数的限制，防止特定极端输入导致的瞬间卡顿。

  5. inject_text 与 lookup 的调用冗余
  在 crates/engine/src/processor/mod.rs 中：
   * 分析： inject_text 会调用 lookup(ctx)，而 lookup 内部已经包含了 Compositor::check_auto_commit(ctx)。但在
     inject_text 的结尾，又手动调用了一次 Compositor::update_phantom_action(ctx) 和 check_auto_commit（在某些路径下）。
   * 建议： 统一 Action 的产生路径，确保每个按键事件只触发一次完整的搜索-联想-提交检查流程，减少重复计算。

  6. Slint 窗口层级与焦点争夺 (Wayland)
  在 crates/ui/src/wayland_layer.rs 中：
   * 观察： 使用了 layer-shell 协议。
   * 风险： 在某些窗口管理器（如 Hyprland 或 Sway）上，如果 layer
     设置不当，候选框可能会抢夺焦点，导致正在输入的编辑器失去焦点。
   * 建议： 检查 KeyboardInteractivity 的设置，确保在任何情况下候选框窗口都不会主动索取键盘焦点。

  7. 配置保存的磁盘 I/O 频率
   * 问题： 每次通过托盘或网页切换状态（如切换简繁、状态栏显示）时，都会直接触发 w.save() 同步写入磁盘。
   * 建议： 磁盘写入应改为异步或延迟合并写入，避免用户快速切换设置时产生不必要的磁盘 IO 压力。



1. 核心引擎与逻辑问题 (Engine & Pipeline)

   * 候选项截断 Bug (关键): [已修复]
       * 在 Pipeline::run 中，代码在执行 Filter（如 AdaptiveFilter 调频逻辑）之前，先将候选项截断到了 200 个。
       * 后果： 如果一个词在词典原始排序中排在 200
         名之后，即使你频繁使用它，调频逻辑也永远无法看到它，导致"排名不上升"。
       * 修复： 将 truncate 移到 Filter 循环之后，Filter 前保留所有候选，确保 AdaptiveFilter 能看到所有候选。

   * 用户词典元数据丢失: [已修复]
       * UserDictTranslator 目前只存储了简化字文本。当一个词变成"用户词"后，它丢失了繁体、辅助码、音调和简拼信息。
       * 后果： 导致"原来的简拼没了"或"辅助码不匹配"。
       * 修复： 给 UserDictTranslator 添加 trie 引用，查询时从词典中提取繁体、辅助码、笔画码。

   * 调频算法权重不足:
       * 当前的 AdaptiveFilter 使用的 RECENCY_BOOST_BASE (5M) 可能不足以抵消 MatchLevelScoringFilter
         给出的基础权重差距（如精确匹配 vs 模糊匹配的巨大分差）。
       * 改进： 需要根据用户反馈动态调整权重曲线，确保高频词能稳居首位。

   * 107 个候选只显示 95 个的问题: [已修复]
       * 这通常是由于 Pipeline 中多处 truncate 或 retain（去重）逻辑不一致导致的。主要原因是候选项截断Bug，
         Filter 前截断到 200 导致候选数量减少。放宽 truncate 后此问题应解决。

   * 自动组句和简拼结果不应进入用户词典: [已修复]
       * record_usage() 没有区分候选来源。ComposeTranslator 的自动组句结果和 TableTranslator 的简拼匹配结果
         （source="Compose" 和 source="Table (Abbr)"）不应进入 learned_words。
       * 修复： 在 record_usage() 中增加 source 过滤，只有精确匹配（非自动组句、非简拼）才能进入用户词典。

   * Pipeline::run retain 去重仅用 text 字段，可能丢失高质量匹配:
       * retain 使用 HashSet<String> 基于 text 去重，如果同一文字来自不同匹配级别（如模糊 vs 精确）或
         不同来源，后出现的版本被丢弃。当前 translator 顺序（UserDict → Table → Compose）部分缓解了此问题，
         但仍有边缘情况。
       * 建议： 改为基于 (text, source) 或 (text, match_level) 去重。

   * Config::save() 全局锁与 RwLock<Config> 不一致:
       * Config::save() 内部使用 OnceLock<Mutex<()>> 做序列化保护，但 Config 本身已被 RwLock<Config>
         保护。两层锁机制存在一致性问题，save() 写入时读端可能看到中间状态。
       * 建议： 统一使用单一锁策略，或确保 save() 在持有写锁时执行。

   * Config::save() 在异步上下文中执行阻塞 I/O:
       * web.rs 的 API handler 在 tokio 异步上下文中直接调用 Config::save()，这是同步阻塞的文件写入操作，
         会阻塞 tokio worker 线程。
       * 建议： 使用 tokio::task::spawn_blocking 包裹 save() 调用。

       

   UI 与交互问题 (Slint & GUI)

   * Slint 宽度适配:
       * candidate.slint 的 width 虽然设置了 max(main_rect.preferred-width, 200px)，但在某些 Window Manager 下（尤其是
         Wayland），窗口的初始约束可能限制了其自动伸缩。
       * 改进： 在 Rust 层根据候选项文本长度动态计算最小宽度并推送给 Slint。
