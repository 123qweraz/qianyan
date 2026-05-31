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

   ---

   2. 渲染后端选择：为何使用软件渲染器而非 Skia

   问题分析：
   候选框中的 emoji 显示为单色轮廓而非彩色，根本原因是 Slint 软件渲染器（SoftwareRenderer）的字体光栅化
   只使用 `swash::scale::Source::Outline` + `Format::Alpha`，将每个字形渲染为单通道 Alpha 遮罩，
   然后用文本颜色着色。COLR/CPAL 彩色表从未被访问，因此无法输出彩色像素。

   为什么不切回 Skia？

   历史背景：
   - 项目早期确实使用 Skia（通过 Slint 的 renderer-skia-opengl 后端），但当时发现离屏渲染场景下
     Skia 需要创建 EGL/OpenGL 上下文，候选框每次按键都要重绘，GL 上下文切换开销导致明显卡顿。
   - 当时的解决方案是切到 fontdue（一个纯 CPU 字体光栅化库），极大降低了渲染延迟。
   - Slint 1.5+ 后，软件渲染器内部用 `swash` + `skrifa` 替换了 fontdue，但仍属于纯 CPU 光栅化。

   当前权衡：

   | 方案 | 优点 | 缺点 |
   |------|------|------|
   | 软件渲染器（swash+skrifa） | 纯 CPU，无 GL 依赖，离屏渲染直接在内存 buffer 中完成，延迟低 | 不支持彩色 emoji（仅单色轮廓） |
   | Skia（renderer-skia-opengl） | 原生支持彩色 emoji（COLR/CPAL） | 需要 headless GL context + wl_egl_window，离屏渲染复杂；每次重绘有 GL 上下文开销，可能卡顿；依赖 glutin/glow 等重量级库 |

   决策：维持软件渲染器，不切 Skia。

   理由：
   1. 候选框 95%+ 内容是中文文本，emoji 出现频率极低。
   2. 单色 emoji 轮廓已能辨识图形，不影响选词功能。
   3. Skia 的 GL 上下文在离屏频繁重绘场景下，历史上有过卡顿问题，不是纯软件渲染器的直接替代品。
   4. 若要修复彩色 emoji，更轻量的做法是 fork/patch i-slint-renderer-software，在 render_vector_glyph
      中增加 `Source::ColorOutline` 回退 + RGBA 输出。但这目前不值得做。

   如果未来确实需要彩色 emoji：
   - 优先 patch 软件渲染器（改动量 ~200 行），而非切换到 Skia 渲染器。
   - 路线：修改 vectorfont.rs 的 render_vector_glyph，使用 `[Source::ColorOutline(0), Source::Outline]`
     和 `Format::Color`，同时扩展 RenderableVectorGlyph 以携带 RGBA 数据。

   ---

  3. Wayland 下候选框显示不全

  问题分析：
  Wayland 的 layer-shell 协议对窗口位置和大小有严格限制。如果 CandidateWindow 的尺寸计算与 LayerSurface
  的配置不一致，或者锚点（Anchor）设置错误，会导致窗口被裁剪。

  解决方案：
   1. 检查 crates/ui/src/wayland_layer.rs：
       * 确保 set_size 调用时传入的是逻辑像素，并且与 Slint 内部计算的 window_width/height 同步。
   2. 动态调整锚点：
       * 根据输入位置计算窗口应该向左还是向右展开。避免窗口超出屏幕边缘。
   3. Size Constraints：
       * 在创建 LayerSurface 时，显式设置 set_keyboard_interactivity(KeyboardInteractivity::None)
         以免干扰输入，并确保 Layer::Overlay 等级足够高。

  ---

  4. 切换 UI 显示模式导致软件崩溃

  问题分析：
  根源在于 ApplyConfig 处理逻辑。在 crates/ui/src/gui_slint.rs 中，修改显示模式会调用 display.close()。而
  SlintDisplay::close() 隐藏了作为 "永久锚点" 的状态栏窗口。Slint
  检测到没有可见窗口后会自动退出事件循环（Event Loop），导致 GUI 进程及其父进程关闭。

  解决方案：
   1. 避免重建：
       * 修改 crates/ui/src/gui_slint.rs。如果只是切换 "候选词 UI 显示模式"（即 show_slint_window
         变化），不要调用 displays.clear() 重建。
       * 直接调用 display.apply_config(config)，让 SlintDisplay 内部通过透明度或 candidate_enabled
         标志自行控制显隐。
   2. 保持窗口存活：
       * 如果必须重建，应先创建新窗口，再关闭旧窗口，确保事件循环中始终有一个窗口是 Active 状态。

  ---

  5. 彻底移除状态栏 (StatusBar)

  问题分析：
  状态栏目前不仅用于显示状态，还充当了防止事件循环退出的 "锚点窗口"。直接删除会导致程序 hide
  候选框时直接退出。

  解决方案：
   1. 清理 UI：
       * 删除 crates/ui/src/main.slint 中的 StatusBar 定义。
       * 删除 crates/ui/src/status_bar.slint 文件。
   2. 重构锚点逻辑：
       * 在 crates/ui/src/slint_window.rs 中，将 CandidateWindow 设为主要窗口。
       * 为了防止 hide 时退出，可以使用一个 1x1 像素且完全透明的不可见窗口作为永久存活的锚点。
   3. 清理配置项：
       * 从 configs/system.json 和 Web 配置界面（static/ 目录）中删除所有关于 "显示状态栏" 的开关和选项。
       * 删除托盘菜单 crates/ui/src/tray.rs 中的 "显示/隐藏状态栏" 菜单项。

  ---

  6. 使用托盘图标显示中英状态

  问题分析：
  用户无法通过托盘直接判断当前是中文还是英文模式。

  解决方案：
   1. 准备素材：
       * 准备两张不同颜色的图标，例如 icon_zh.png (彩色) 和 icon_en.png (灰色)。
   2. 更新托盘逻辑：
       * 在 crates/ui/src/tray.rs 中增加一个 update_status(chinese_enabled: bool) 方法。
       * Windows： 调用 Shell_NotifyIconW 使用新的 hIcon 句柄进行 NIM_MODIFY。
       * Linux (KSNI)： 更新 ImeTray 结构体中的状态，触发 icon_pixmap 的重新生成。
   3. 联动：
       * 在 src/main.rs 处理 MainToGui::ShowStatus 事件时，同步向托盘发送更新图标的消息。

  ---

  7. 中英混输优化

  建议方向：

