
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
