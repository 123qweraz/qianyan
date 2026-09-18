//! 快捷键回归测试（空词典环境，纯状态机逻辑）：
//! - 单独轻按 Shift 切换中/英文
//! - Ctrl+Shift+F 繁简体切换（默认绑定，可在设置中修改）
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
fn ctrl_shift_f_toggles_traditional() {
    let mut p = new_proc();
    assert!(!p.ctx.session_state.traditional_enabled);

    // handle_key(key, val, shift, ctrl, alt)
    let act = p.handle_key(VirtualKey::F, PRESS, true, true, false);
    assert!(
        matches!(act, Action::Notify(ref s, _) if s == "繁體"),
        "应通知切到繁体，得到 {:?}",
        act
    );
    assert!(p.ctx.session_state.traditional_enabled);

    let act = p.handle_key(VirtualKey::F, PRESS, true, true, false);
    assert!(
        matches!(act, Action::Notify(ref s, _) if s == "简体"),
        "应通知切回简体，得到 {:?}",
        act
    );
    assert!(!p.ctx.session_state.traditional_enabled);
}

#[test]
fn ctrl_shift_f_with_non_empty_buffer_does_not_close_ime() {
    let mut p = new_proc();
    p.ctx.session_state.chinese_enabled = true;
    let _ = p.handle_key(VirtualKey::A, PRESS, false, false, false);
    assert!(!p.ctx.session.buffer.is_empty());

    let act = p.handle_key(VirtualKey::F, PRESS, true, true, false);
    assert!(
        matches!(act, Action::Notify(ref s, _) if s == "繁體"),
        "应通知切到繁体，得到 {:?}",
        act
    );
    assert!(p.ctx.session_state.chinese_enabled, "繁简切换不得关闭 IME");
}

#[test]
fn plain_f_does_not_toggle_traditional() {
    let mut p = new_proc();
    p.ctx.session_state.chinese_enabled = true;

    let act = p.handle_key(VirtualKey::F, PRESS, false, false, false);
    assert!(!p.ctx.session_state.traditional_enabled);
    assert!(
        !matches!(act, Action::Notify(..)),
        "普通 F 不应触发繁简切换，得到 {:?}",
        act
    );
}

#[test]
fn tap_shift_toggles_language() {
    let mut p = new_proc();
    let before = p.ctx.session_state.chinese_enabled;

    let _ = p.handle_key(VirtualKey::Shift, PRESS, true, false, false);
    let release = p.handle_key(VirtualKey::Shift, RELEASE, false, false, false);
    assert!(
        matches!(release, Action::Consume),
        "单独 Shift 释放应被消费，得到 {:?}",
        release
    );
    assert_eq!(p.ctx.session_state.chinese_enabled, !before);
}

#[test]
fn shift_with_other_key_does_not_toggle_language() {
    let mut p = new_proc();
    p.ctx.session_state.chinese_enabled = true;

    let _ = p.handle_key(VirtualKey::Shift, PRESS, true, false, false);
    let _ = p.handle_key(VirtualKey::A, PRESS, true, false, false);
    let _ = p.handle_key(VirtualKey::Shift, RELEASE, false, false, false);
    assert!(
        p.ctx.session_state.chinese_enabled,
        "Shift 参与组合键时不应切换语言"
    );
}

#[test]
fn shift_tap_in_english_mode_switches_back_to_chinese() {
    let mut p = new_proc();
    p.ctx.session_state.chinese_enabled = false;

    let _ = p.handle_key(VirtualKey::Shift, PRESS, true, false, false);
    let release = p.handle_key(VirtualKey::Shift, RELEASE, false, false, false);
    assert!(
        matches!(release, Action::Consume),
        "英文模式下单独 Shift 释放也应被消费，得到 {:?}",
        release
    );
    assert!(p.ctx.session_state.chinese_enabled);
}
