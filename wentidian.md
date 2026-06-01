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
