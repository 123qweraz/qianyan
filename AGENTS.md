你在网页加的开关没用啊，而且和用户词典的启用频率学习，貌似冲突了我把 `/.confing/qian-ime/user_data/chinese 的几个json词典删了，才恢复正常，话说流输入法的算法是怎么设计的，我们能直接抄他们的算法就好了，我觉得你写得有问题啊，怎么我打某个单字时候，这个排序算法没起作用，不像词组，我打一下，下次打，立刻在首位，而比如说 da答 打了10次， 大da 还是在首位，是怎么回事，难道算法只对词组作用，对单字无用，还是因为单字词典的频率和词组不同，怎么排序算法对词组有用，对单字无用


核心改进：
   1. 新增过滤模式：将原有的“生僻字开关”升级为“生僻字模式”，支持以下三种状态：
       * 只显示常用字（默认）：隐藏所有生僻字。
       * 显示所有：同时显示常用字和生僻字。
       * 只显示生僻字：过滤掉常用字，仅显示 Level 4 和 Level 5 的生僻字符。
   2. 性能优化：通过在 Candidate 候选项结构体中增加 flags
      字段，直接在检索过程中携带“是否为生僻字”的元数据。这样在过滤时，引擎不再需要进行昂贵的 HashSet
      集合查询，而是通过极快的位运算完成过滤。
   3. UI 升级：在拼音设置页面将开关替换为下拉选择框，方便用户随时切换模式。

  ---

  计划：增强生僻字逻辑的灵活性

  1. 配置层重构 (Config Refactoring)
   * 在 crates/core/src/config.rs 中引入 RareCharMode 枚举（common_only, include_rare, only_rare）。
   * 在 Input 配置中增加 rare_char_mode 字段。
   * 在 Config::load 中增加兼容逻辑，确保旧版本的 enable_rare_chars: true 自动映射为 IncludeRare。

  2. 候选项元数据增强 (Candidate Metadata)
   * 为 Candidate 和 SchemeCandidate 结构体增加 flags: u8 字段。
   * bit 0 (0x01) 定义为生僻字标志位。

  3. 数据流打通 (Data Flow)
   * 修改 ChineseScheme 和各路 Translator，在从字典提取结果时，将 TrieResult 的原始 flags 透传给 Candidate。

  4. 过滤逻辑优化 (Filtering Logic)
   * 在 SearchEngine::search 中，根据 rare_char_mode 直接对 candidates 进行 retain
     操作，不再依赖外部加载的生僻字集合。

  5. UI 与设置界面更新 (UI Update)
   * 更新 static/settings/pinyin.html，将开关改为下拉菜单，并绑定新配置项。

  6. 验证 (Verification)
   * 通过重编词库验证 flags 是否正确携带。
   * 实测 OnlyRare 模式下输入 han 是否能精准且仅过滤出生僻候选词。
