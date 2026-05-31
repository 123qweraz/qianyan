
编辑功能， tab ＋h ＋l，改成左右移动光标
增加更多的光标移动功能，tab＋a 移动到字母行首，tab＋e移动到字母行尾， w一个个音节移动， dd删掉整个拼音，dw删掉一个音节，
编辑功能，i插入，r替换，i是在光标插入字母，r是替换光标出字母，有点想backspace和delete的差别

slint bug
slint emoji不会显示成彩色的，而是黑白的
Emoji 颜色:
       * 这是由于 Slint 在某些 Linux 环境下默认不加载彩色字体库。需要显式配置字体回退路径或使用支持彩色 Emoji
         的渲染后端。
在wayland时，候选栏显示不全，在x11正常，但是我不清楚原因
候选词UI显示模式
选择候选词的显示方式
这个设置有问题，每次切换同步UI显示模式，都会导致软件关闭，然后我手动重新打开，会应用上次保存的设置。看看是怎么回事，怎么切换一个显示模式，会导致整个软件关闭呢
问题分析
根因在 crates/ui/src/gui_slint.rs 的 ApplyConfig 事件处理中。
触发路径
1. 用户在 system.html 切换"候选词UI显示模式" → 保存
2. Web服务器调用 tray_tx.send(TrayEvent::ReloadConfig) 
3. 主进程收到后，发送 GuiEvent::ApplyConfig 给GUI进程
4. GUI进程的 handle_ipc_event / handle_event 检测到 show_slint_window 或 show_notification 等标志变化
崩溃原因
在 gui_slint.rs:244-253 中：
if new_slint != old_slint || new_notify != old_notify || new_toggle_notify != old_toggle_notify {
    for display in displays.iter_mut() {
        display.close();    // ← 第1步: 隐藏所有窗口（包括状态栏锚点窗口）
    }
    displays.clear();       // ← 第2步: 销毁所有Slint窗口句柄
    *displays = create_displays(config);  // ← 第3步: 重新创建
}
关键问题是 SlintDisplay::close() (crates/ui/src/slint_window.rs:412-416)：
fn close(&mut self) {
    let _ = self.window.window().hide();
    let _ = self.status_bar.window().hide();  // ← 连锚点窗口也隐藏了！
    self.window_visible = false;
}
注释在 slint_window.rs:160-162 明确说明状态栏是 *"永久锚点窗口（始终 show），防止...因无窗口而自动退出 event loop"*。但 close() 违背了这个设计，强行隐藏了状态栏。
然后 displays.clear() 会 drop 掉 SlintDisplay 和它内部的 CandidateWindow + StatusBar 组件句柄。当所有 Slint 窗口句柄都被销毁时，Slint 的 winit 后端检测到没有窗口了，就会退出事件循环 (slint::run_event_loop() 返回)，导致 GUI 进程退出。主进程等 GUI 退出后也退出 => 整个软件关闭。
即使 create_displays() 会新建窗口，但旧的窗口销毁已经触发了事件循环退出，来不及了。
修复方向
方案 A：不为 show_slint_window 的变化而重建 SlintDisplay/WaylandLayerDisplay——它们本来就不依赖这个标志来决定是否创建（create_displays 总是创建 SlintDisplay/WaylandLayerDisplay）。只需要调 apply_config() 更新 candidate_enabled 字段即可。只有 show_notification 变化时才需要增删 LinuxNotifyDisplay。
方案 B：如果一定要重建，应该先创建新窗口再销毁旧窗口，保证事件循环始终有窗口存活。
推荐方案 A，改动最小且最安全。

删掉状态栏，过括托盘与网页的候选栏关于状态栏的相关设置

用不同颜色的托盘图标，显示输入法状态

中英混输 优化， 还没想好




