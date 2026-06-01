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
       * Tab + H/L → 左右移动光标。
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
