//! CapsLock+F 繁简切换的回归测试（空词典环境，纯状态机逻辑）
use qianyan_ime_engine::keys::VirtualKey;
use qianyan_ime_engine::processor::Action;
use qianyan_ime_engine::Processor;

fn new_proc() -> Processor {
    Processor::new(
        std::collections::HashMap::new(),
        std::collections::HashMap::new(),
    )
}

const PRESS: i32 = 1;
const RELEASE: i32 = 0;

#[test]
fn capslock_f_with_empty_buffer_switches_language_back_and_toggles_traditional() {
    let mut p = new_proc();

    // 初始为英文模式（chinese_enabled=false）
    assert!(!p.ctx.session_state.chinese_enabled);

    // 空 buffer 按 CapsLock：切换语言 + 允许 F 撤销
    assert!(matches!(p.handle_key(VirtualKey::CapsLock, PRESS, false, false, false), Action::Consume));
    assert!(p.ctx.session_state.chinese_enabled, "空 buffer CapsLock 应切换语言");
    assert!(p.ctx.session_state.capslock_switched_language);

    // 按住 CapsLock 时按 F：撤销语言切换 + 打开繁体
    let act = p.handle_key(VirtualKey::F, PRESS, false, false, false);
    assert!(matches!(act, Action::Notify(ref s, _) if s == "繁體"), "应通知切到繁体，得到 {:?}", act);
    assert!(!p.ctx.session_state.chinese_enabled, "CapsLock+F 应撤销语言切换回到英文");
    assert!(p.ctx.session_state.traditional_enabled);
}

#[test]
fn capslock_f_with_non_empty_buffer_does_not_close_ime() {
    let mut p = new_proc();

    // 先进入中文模式（空 buffer 按 CapsLock）
    assert!(matches!(p.handle_key(VirtualKey::CapsLock, PRESS, false, false, false), Action::Consume));
    assert!(p.ctx.session_state.chinese_enabled);
    p.handle_key(VirtualKey::CapsLock, RELEASE, false, false, false);

    // 打字符建立非空 buffer
    let act = p.handle_key(VirtualKey::A, PRESS, false, false, false);
    assert!(!matches!(act, Action::PassThrough), "中文模式下字母不应直通: {:?}", act);
    assert!(!p.ctx.session.buffer.is_empty());

    // 非空 buffer 按 CapsLock：不切换语言（只记 capslock_down）
    assert!(matches!(p.handle_key(VirtualKey::CapsLock, PRESS, false, false, false), Action::Consume));
    assert!(p.ctx.session_state.chinese_enabled, "非空 buffer 不应切换语言");
    assert!(!p.ctx.session_state.capslock_switched_language);

    // CapsLock+F：只应切繁体，绝不能把 IME 关掉
    let act = p.handle_key(VirtualKey::F, PRESS, false, false, false);
    assert!(matches!(act, Action::Notify(ref s, _) if s == "繁體"), "应通知切到繁体: {:?}", act);
    assert!(p.ctx.session_state.chinese_enabled, "非空 buffer 的 CapsLock+F 不得关闭 IME");

    // 中文模式仍在：字母按键不应直通
    let act = p.handle_key(VirtualKey::B, PRESS, false, false, false);
    assert!(!matches!(act, Action::PassThrough), "中文模式应继续保持: {:?}", act);
}