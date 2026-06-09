use crate::compositor::{should_block_invalid_input, Compositor};
use crate::keys::VirtualKey;
use crate::pipeline::lookup;
use crate::processor::commands;
use crate::processor::utils::*;
use crate::processor::{Action, Command, FilterMode};
use crate::EngineContext;
use std::time::Instant;

pub fn handle_idle(
    ctx: &mut EngineContext,
    key: VirtualKey,
    shift_pressed: bool,
    perform_lookup: bool,
) -> Action {
    if key == VirtualKey::Enter || key == VirtualKey::Space {
        return Action::PassThrough;
    }

    if is_digit(key) {
        if let Some(c) = key_to_char(key, shift_pressed, ctx.session_state.caps_lock_enabled) {
            return Action::Emit(c.to_string());
        }
    }

    if is_letter(key) {
        if let Some(c) = key_to_char(key, shift_pressed, ctx.session_state.caps_lock_enabled) {
            let lang = ctx
                .session_state
                .active_profiles
                .first()
                .cloned()
                .unwrap_or_default()
                .to_lowercase();

            if let Some(layout) = ctx.config.layouts().get(&lang) {
                if let Some(action) = layout.mappings.get(&c.to_string()) {
                    if shift_pressed && !action.shift.is_empty() {
                        return Action::Emit(action.shift.clone());
                    } else if !action.tap.is_empty() {
                        return Action::Emit(action.tap.clone());
                    }
                }
            }

            if let Some(layout) = ctx.config.keyboard_layouts().get(&lang) {
                if let Some(mapped) = layout.get(&c.to_string()) {
                    return Action::Emit(mapped.clone());
                }
            }

            ctx.session.push_char(c);
            if perform_lookup {
                if let Some(act) = lookup(ctx) {
                    return act;
                }
            }
            if should_block_invalid_input(ctx, &ctx.session.buffer.clone()) {
                return Action::Alert;
            }
            let _ = Compositor::update_phantom_action(ctx);
        } else {
            log::warn!("[handle_idle] key_to_char returned None for key={:?}", key);
        }
    }

    if get_punctuation_key(key, shift_pressed).is_some() {
        return handle_punctuation(ctx, key, shift_pressed);
    }

    Action::PassThrough
}

pub fn handle_composing(
    ctx: &mut EngineContext,
    key: VirtualKey,
    shift_pressed: bool,
    perform_lookup: bool,
) -> Action {
    let mods = crate::ModifierState {
        shift: shift_pressed,
        ctrl: false,
        alt: false,
        meta: false,
    };

    if let Some(cmd) = ctx.dispatcher.key_map.get(&(key, mods)).cloned() {
        let final_cmd = if ctx.config.swap_arrow_keys() {
            match (key, cmd.clone()) {
                (VirtualKey::Up, Command::PrevPage) => Command::PrevCandidate,
                (VirtualKey::Down, Command::NextPage) => Command::NextCandidate,
                (VirtualKey::Left, Command::PrevCandidate) => Command::PrevPage,
                (VirtualKey::Right, Command::NextPage) => Command::NextPage,
                _ => cmd,
            }
        } else {
            cmd
        };

        if key == VirtualKey::Space && shift_pressed {
            if let Some(cand) = ctx.session.candidates.get(ctx.session.selected) {
                if !cand.hint.is_empty() {
                    return commands::commit_candidate(ctx, cand.hint.clone(), 99);
                }
            }
        }
        return commands::execute_command(ctx, final_cmd);
    }

    if ctx.session.nav_mode {
        match key {
            VirtualKey::H => return commands::execute_command(ctx, Command::PrevCandidate),
            VirtualKey::L => return commands::execute_command(ctx, Command::NextCandidate),
            VirtualKey::K => return commands::execute_command(ctx, Command::PrevPage),
            VirtualKey::J => return commands::execute_command(ctx, Command::NextPage),
            _ => {}
        }
        // 导航编辑键（可通过配置修改）
        if ctx.config.nav_delete_keys().contains(&key) {
            ctx.session.delete_at_cursor();
            return lookup(ctx).unwrap_or(Action::Consume);
        }
        if ctx.config.nav_clear_keys().contains(&key) {
            ctx.session.clear_buffer();
            return Action::Consume;
        }
    }

    // 键盘布局映射：在拼音输入中也生效（nav_mode 优先）
    if is_letter(key) {
        if let Some(c) = key_to_char(key, false, ctx.session_state.caps_lock_enabled) {
            let lang = ctx
                .session_state
                .active_profiles
                .first()
                .cloned()
                .unwrap_or_default()
                .to_lowercase();
            if let Some(layout) = ctx.config.layouts().get(&lang) {
                if let Some(action) = layout.mappings.get(&c.to_string()) {
                    if shift_pressed && !action.shift.is_empty() {
                        return Action::Emit(action.shift.clone());
                    } else if !shift_pressed && !action.tap.is_empty() {
                        return Action::Emit(action.tap.clone());
                    }
                }
            }
            if !shift_pressed {
                if let Some(mapped) = ctx.config.keyboard_layouts().get(&lang).and_then(|m| m.get(&c.to_string())) {
                    return Action::Emit(mapped.clone());
                }
            }
        }
    }

    if is_letter(key) && shift_pressed && !ctx.session.buffer.is_empty()
        && ctx.session.has_dict_match
        && ctx.session.buffer.chars().any(|c| c.is_ascii_lowercase())
    {
        if let Some(c) = key_to_char(key, false, ctx.session_state.caps_lock_enabled) {
            ctx.session.shift_used_as_modifier = true;
            if ctx.session.filter_mode != FilterMode::Global {
                ctx.session.filter_mode = FilterMode::Global;
            }
            ctx.session.handle_filter_char(c);

            if let Some(act) = lookup(ctx) {
                return act;
            }
            return Compositor::update_phantom_action(ctx);
        }
    }

    let has_cand = !ctx.session.candidates.is_empty();
    let now = Instant::now();

    let current_profile = ctx
        .session_state
        .active_profiles
        .first()
        .cloned()
        .unwrap_or_default();
    if let Some(scheme) = ctx.engine.schemes.get(&current_profile) {
        let mut tries_map = std::collections::HashMap::with_capacity(ctx.session_state.active_profiles.len());
        for profile in &ctx.session_state.active_profiles {
            if let Some(pipeline) = ctx.engine.get_or_create_pipeline(profile) {
                if let Some(trie) = ctx.engine.get_trie_from_pipeline(pipeline.as_ref()) {
                    tries_map.insert(profile.clone(), trie.clone());
                }
            }
        }
        let last_word = ctx
            .session_state
            .commit_history
            .last()
            .map(|(_, word)| word.as_str());
        let last_two = ctx
            .session_state
            .get_last_two_words();
        let context = crate::scheme::SchemeContext {
            config: &ctx.config.master_config,
            tries: &tries_map,
            syllable_freq: &ctx.engine.syllable_freq,
            base_syllables: &ctx.engine.base_syllables,
            single_syllables: &ctx.engine.single_syllables,
            user_dict: &ctx.config.combined_dict,
            ngram_history: &ctx.engine.ngram_history,
            user_order: &ctx.config.user_order,
            active_profiles: &ctx.session_state.active_profiles,
            candidate_count: ctx.session.candidates.len(),
            last_word,
            last_two_words: last_two,
            _filter_mode: ctx.session.filter_mode.clone(),
            _aux_filter: &ctx.session.aux_filter,
        };
        let act_opt: Option<Action> =
            scheme.handle_special_key(key, &mut ctx.session.buffer, &context);
        if let Some(act) = act_opt {
            if act == Action::Consume {
                if perform_lookup {
                    if let Some(lookup_act) = lookup(ctx) {
                        return lookup_act;
                    }
                }
                return Compositor::update_phantom_action(ctx);
            }
            return act;
        }
    }

    if is_letter(key) {
        if ctx.session.filter_mode != FilterMode::None {
            if let Some(c) = key_to_char(key, false, ctx.session_state.caps_lock_enabled) {
                ctx.session.handle_filter_char(c);
                if perform_lookup {
                    if let Some(act) = lookup(ctx) {
                        return act;
                    }
                }
                return Compositor::update_phantom_action(ctx);
            }
        }

        if !shift_pressed && ctx.config.enable_double_tap() {
            if let Some(last_k) = ctx.dispatcher.last_tap_key {
                if last_k == key {
                    if let Some(last_t) = ctx.dispatcher.last_tap_time {
                        if now.duration_since(last_t) <= ctx.config.double_tap_timeout() {
                            if let Some(c) =
                                key_to_char(key, false, ctx.session_state.caps_lock_enabled)
                            {
                                let lang = ctx
                                    .session_state
                                    .active_profiles
                                    .first()
                                    .cloned()
                                    .unwrap_or_default()
                                    .to_lowercase();

                                let mut replacement = None;
                                if let Some(layout) = ctx.config.layouts().get(&lang) {
                                    if let Some(action) = layout.mappings.get(&c.to_string()) {
                                        if let Some(dt) = &action.double_tap {
                                            replacement = Some(dt.clone());
                                        }
                                    }
                                }

                                if replacement.is_none() {
                                    replacement =
                                        ctx.config.double_taps().get(&c.to_string()).cloned();
                                }

                                // 双击出大写：如果没找到常规双击映射，检查是否启用了双击大写功能
                                if replacement.is_none() {
                                    let uppercase_keys = ctx.config.double_tap_uppercase_keys();
                                    if uppercase_keys.contains(&c.to_string()) {
                                        ctx.dispatcher.last_tap_key = None;
                                        ctx.dispatcher.last_tap_time = None;
                                        // 去掉第一击多输入的字母
                                        if ctx.session.buffer.ends_with(c) {
                                            ctx.session.buffer.pop();
                                        }
                                        // 空缓冲区不触发过滤（如单独双击某个字母）
                                        if ctx.session.buffer.is_empty() {
                                            ctx.session.clear_buffer();
                                            return Compositor::update_phantom_action(ctx);
                                        }
                                        // 模拟 Shift+key：触发辅码过滤模式
                                        ctx.session.shift_used_as_modifier = true;
                                        if ctx.session.filter_mode != FilterMode::Global {
                                            ctx.session.filter_mode = FilterMode::Global;
                                        }
                                        ctx.session.handle_filter_char(c);
                                        if perform_lookup {
                                            if let Some(act) = lookup(ctx) {
                                                return act;
                                            }
                                        }
                                        return Compositor::update_phantom_action(ctx);
                                    }
                                }

                                if let Some(r) = replacement {
                                    if ctx.session.buffer.ends_with(c) {
                                        ctx.session.buffer.pop();
                                        ctx.session.buffer.push_str(&r);
                                        ctx.dispatcher.last_tap_key = None;
                                        ctx.dispatcher.last_tap_time = None;
                                        if perform_lookup {
                                            if let Some(act) = lookup(ctx) {
                                                return act;
                                            }
                                        }
                                        return Compositor::update_phantom_action(ctx);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            ctx.dispatcher.last_tap_key = Some(key);
            ctx.dispatcher.last_tap_time = Some(now);
        } else {
            ctx.dispatcher.last_tap_key = None;
            ctx.dispatcher.last_tap_time = None;
        }

        if let Some(c) = key_to_char(key, shift_pressed, ctx.session_state.caps_lock_enabled) {
            if shift_pressed {
                ctx.session.shift_used_as_modifier = true;
            }
            ctx.session.push_char(c);
            if perform_lookup {
                if let Some(act) = lookup(ctx) {
                    return act;
                }
            }
            if should_block_invalid_input(ctx, &ctx.session.buffer.clone()) {
                return Action::Alert;
            }
            if let Some(act) = Compositor::check_auto_commit(ctx) {
                return act;
            }
            return Compositor::update_phantom_action(ctx);
        }
    } else {
        ctx.dispatcher.last_tap_key = None;
        ctx.dispatcher.last_tap_time = None;
    }

    if has_cand && ctx.session.has_dict_match {
        if ctx.session_state.tab_down {
            for (vk, pos) in ctx.config.word_to_char_shift_keys() {
                if vk == key {
                    return commands::execute_command(ctx, Command::SelectChar(pos));
                }
            }
        } else {
            for (vk, pos) in ctx.config.word_to_char_keys() {
                if vk == key {
                    return commands::execute_command(ctx, Command::SelectChar(pos));
                }
            }
        }
    }

    if ctx.config.page_up_keys().contains(&key) && has_cand {
        return commands::execute_command(ctx, Command::PrevPage);
    }
    if ctx.config.page_down_keys().contains(&key) && has_cand {
        return commands::execute_command(ctx, Command::NextPage);
    }
    if ctx.config.prev_candidate_keys().contains(&key) && has_cand {
        return commands::execute_command(ctx, Command::PrevCandidate);
    }
    if ctx.config.next_candidate_keys().contains(&key) && has_cand {
        return commands::execute_command(ctx, Command::NextCandidate);
    }

    if key == VirtualKey::Semicolon && !shift_pressed {
        ctx.session.push_char(';');
        if perform_lookup {
            if let Some(act) = lookup(ctx) {
                return act;
            }
        }
        return Compositor::update_phantom_action(ctx);
    }

    match key {
        VirtualKey::Backspace => {
            // CapsLock + Backspace: delete entire last syllable
            if ctx.session_state.capslock_down && !ctx.session.buffer.is_empty() {
                let delete_count = if let Some(last_seg) = ctx.session.best_segmentation.last() {
                    last_seg.chars().count()
                } else {
                    let buffer = &ctx.session.buffer;
                    let mut count = 1;
                    for len in (2..=buffer.len().min(6)).rev() {
                        let suffix = &buffer[buffer.len() - len..];
                        if ctx.engine.syllable_freq.contains_key(suffix) {
                            count = len;
                            break;
                        }
                    }
                    count
                };

                let old_phantom_len = ctx.session.phantom_text.chars().count();
                for _ in 0..delete_count {
                    ctx.session.pop_char();
                }

                if ctx.session.buffer.is_empty() {
                    ctx.reset();
                    if old_phantom_len > 0 {
                        return Action::DeleteAndEmit {
                            delete: old_phantom_len,
                            insert: "".into(),
                        };
                    }
                    return Action::Consume;
                } else {
                    if perform_lookup {
                        if let Some(act) = lookup(ctx) {
                            return act;
                        }
                    }
                    return Compositor::update_phantom_action(ctx);
                }
            }

            if ctx.session.filter_mode != FilterMode::None {
                ctx.session.pop_filter();
                if perform_lookup {
                    if let Some(act) = lookup(ctx) {
                        return act;
                    }
                }
                return Compositor::update_phantom_action(ctx);
            }

            if ctx.session.buffer.is_empty() {
                ctx.session_state.commit_history.clear();
                return Action::Consume;
            }

            let old_phantom_len = ctx.session.phantom_text.chars().count();
            ctx.session.pop_char();

            if ctx.session.buffer.is_empty() {
                ctx.reset();
                if old_phantom_len > 0 {
                    Action::DeleteAndEmit {
                        delete: old_phantom_len,
                        insert: "".into(),
                    }
                } else {
                    Action::Consume
                }
            } else {
                if perform_lookup {
                    if let Some(act) = lookup(ctx) {
                        return act;
                    }
                }
                Compositor::update_phantom_action(ctx)
            }
        }

        VirtualKey::Home => {
            if shift_pressed {
                ctx.session.selected = 0;
                ctx.session.page = 0;
            } else {
                ctx.session.selected = ctx.session.page;
            }
            Action::Consume
        }
        VirtualKey::End => {
            if has_cand {
                if shift_pressed {
                    ctx.session.selected = ctx.session.candidates.len() - 1;
                    ctx.session.page =
                        (ctx.session.selected / ctx.config.page_size()) * ctx.config.page_size();
                } else {
                    ctx.session.selected = (ctx.session.page + ctx.config.page_size() - 1)
                        .min(ctx.session.candidates.len() - 1);
                }
            }
            Action::Consume
        }

        _ if is_digit(key) => {
            let digit = key_to_digit(key).unwrap_or(0);
            let digit_char = key_to_char(key, false, ctx.session_state.caps_lock_enabled).unwrap_or('0');

            // 有候选词且数字在候选范围内 -> 选词上屏
            if !ctx.session.candidates.is_empty()
                && digit >= 1
                && digit <= ctx.session.candidates.len()
            {
                return commands::execute_command(ctx, Command::Select(digit - 1));
            }

            // 没有候选词或数字超出范围 -> 直接上屏数字字符（像标点符号一样）
            // 清空当前 buffer 和候选词（数字直接上屏，不需要拼音）
            ctx.session.buffer.clear();
            ctx.session.candidates.clear();
            Action::Emit(digit_char.to_string())
        }
        _ => {
            if get_punctuation_key(key, shift_pressed).is_some() {
                handle_punctuation(ctx, key, shift_pressed)
            } else if let Some(c) =
                key_to_char(key, shift_pressed, ctx.session_state.caps_lock_enabled)
            {
                let old_buffer = ctx.session.buffer.clone();
                ctx.session.push_char(c);
                if perform_lookup {
                    if let Some(act) = lookup(ctx) {
                        return act;
                    }
                }
                if should_block_invalid_input(ctx, &old_buffer) {
                    return Action::Alert;
                }
                if let Some(act) = Compositor::check_auto_commit(ctx) {
                    return act;
                }
                Compositor::update_phantom_action(ctx)
            } else {
                Action::PassThrough
            }
        }
    }
}

pub fn handle_punctuation(ctx: &mut EngineContext, key: VirtualKey, shift_pressed: bool) -> Action {
    use crate::processor::punctuation;
    punctuation::handle_punctuation(ctx, key, shift_pressed)
}
