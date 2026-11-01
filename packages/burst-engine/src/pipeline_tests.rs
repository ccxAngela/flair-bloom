//! 引擎管线确定性测试（L1.5）：注入「命令录制」调度器替身，对
//! 「合成物理按键 → 引擎状态机 → 发给调度器的命令（含 generation）」做 golden 断言。
//!
//! 与 `lib.rs` 里的 `active_ids` 测试互补：那些只断言引擎自身状态，这里断言**实际下发到
//! 调度器的命令序列**——能抓住「状态对但命令错/漏/generation 不匹配」这类引擎↔调度契约 bug。
//! 真实注入时序由 `scheduler/sim_tests.rs`（L1）覆盖。

use crate::test_support::{rule, setup};
use qzh_profile::key_id::KeyId;
use qzh_profile::profile::BurstMode;

#[test]
fn hold_press_release_emits_start_then_stop_same_generation() {
    let trigger = KeyId::Keyboard(0x51);
    let (engine, rec) = setup(vec![rule(
        "h",
        BurstMode::Hold,
        trigger,
        KeyId::Keyboard(0x45),
    )]);

    engine.on_key_press(trigger);
    engine.on_key_release(trigger);

    assert_eq!(rec.cmds(), vec!["start:h:g1", "stop:h:g1"]);
}

#[test]
fn wheel_hold_emits_single_tap_not_start_then_stop() {
    // 边界（A1）：滚轮触发的 Hold 规则，每格（press+release 瞬发）只下发一条一次性 tap 命令，
    // 而非 start+stop——后者会被调度器在首拍前合并掉，导致零注入。
    use qzh_profile::key_id::MouseButton;
    let wheel = KeyId::Mouse(MouseButton::WheelUp);
    let (engine, rec) = setup(vec![rule(
        "w",
        BurstMode::Hold,
        wheel,
        KeyId::Keyboard(0x45),
    )]);

    engine.on_key_press(wheel);
    engine.on_key_release(wheel);

    assert_eq!(rec.cmds(), vec!["tap:w:g1"]);
}

#[test]
fn toggle_same_key_starts_then_stops_on_next_press() {
    let trigger = KeyId::Keyboard(0x51);
    let (engine, rec) = setup(vec![rule("t", BurstMode::Toggle, trigger, trigger)]);

    engine.on_key_press(trigger); // 开
    engine.on_key_release(trigger);
    engine.on_key_press(trigger); // 关

    assert_eq!(rec.cmds(), vec!["start:t:g1", "stop:t:g1"]);
}

#[test]
fn rapid_toggle_emits_start_and_stop_even_when_poll_would_see_empty_state() {
    let key = KeyId::Keyboard(0x46);
    let (engine, _) = setup(vec![rule("f", BurstMode::Toggle, key, key)]);
    let feedback = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = feedback.clone();
    engine.set_on_toggle_changed(move |key, started| {
        recorded.lock().unwrap().push((key, started));
    });

    engine.on_key_press(key);
    engine.on_key_release(key);
    engine.on_key_press(key);

    assert!(engine.get_active_ids().is_empty());
    assert_eq!(*feedback.lock().unwrap(), vec![(key, true), (key, false)]);
}

#[test]
fn toggle_group_displacement_and_hold_preemption_do_not_emit_extra_feedback() {
    let a_key = KeyId::Keyboard(0x41);
    let b_key = KeyId::Keyboard(0x42);
    let hold_key = KeyId::Keyboard(0x48);
    let mut a = rule("a", BurstMode::Toggle, a_key, a_key);
    let mut b = rule("b", BurstMode::Toggle, b_key, b_key);
    let mut hold = rule("hold", BurstMode::Hold, hold_key, hold_key);
    for r in [&mut a, &mut b, &mut hold] {
        r.group = Some("g".into());
    }
    let (engine, _) = setup(vec![a, b, hold]);
    let feedback = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorded = feedback.clone();
    engine.set_on_toggle_changed(move |key, started| {
        recorded.lock().unwrap().push((key, started));
    });

    engine.on_key_press(a_key);
    engine.on_key_release(a_key);
    engine.on_key_press(b_key);
    engine.on_key_release(b_key);
    engine.on_key_press(hold_key);
    engine.on_key_release(hold_key);
    engine.on_key_press(b_key);

    assert_eq!(
        *feedback.lock().unwrap(),
        vec![(a_key, true), (b_key, true), (b_key, false)]
    );
}

#[test]
fn toggle_group_displacement_emits_stop_old_then_start_new() {
    let mut a = rule(
        "a",
        BurstMode::Toggle,
        KeyId::Keyboard(0x51),
        KeyId::Keyboard(0x41),
    );
    let mut b = rule(
        "b",
        BurstMode::Toggle,
        KeyId::Keyboard(0x45),
        KeyId::Keyboard(0x42),
    );
    a.group = Some("g".into());
    b.group = Some("g".into());
    let (engine, rec) = setup(vec![a, b]);

    engine.on_key_press(KeyId::Keyboard(0x51)); // 启动 a
    engine.on_key_release(KeyId::Keyboard(0x51));
    engine.on_key_press(KeyId::Keyboard(0x45)); // 顶替 a、启动 b

    // 契约：顶替必须先给调度器发 stop:a，再发 start:b（否则共享/残留会错乱）。
    assert_eq!(rec.cmds(), vec!["start:a:g1", "stop:a:g1", "start:b:g1"]);
}

#[test]
fn disabling_global_switch_issues_stop_all_with_bumped_generation() {
    let trigger = KeyId::Keyboard(0x51);
    let (engine, rec) = setup(vec![rule("t", BurstMode::Toggle, trigger, trigger)]);

    engine.on_key_press(trigger); // toggle 开
    rec.clear();
    engine.set_global_enabled(false, false); // 暂停 → stop_all（generation 递增到 2）

    assert_eq!(rec.cmds(), vec!["stopall:g2"]);
}

#[test]
fn direct_global_changes_and_toggle_feedback_share_state_transition_order() {
    let key = KeyId::Keyboard(0x46);
    let (engine, _) = setup(vec![rule("f", BurstMode::Toggle, key, key)]);
    let events = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let global_events = events.clone();
    engine.set_on_global_changed(move |enabled| {
        global_events
            .lock()
            .unwrap()
            .push(format!("global:{enabled}"));
    });
    let toggle_events = events.clone();
    engine.set_on_toggle_changed(move |_, started| {
        toggle_events
            .lock()
            .unwrap()
            .push(format!("toggle:{started}"));
    });

    engine.set_global_enabled(false, false);
    engine.set_global_enabled(true, false);
    engine.on_key_press(key);
    engine.on_key_release(key);
    engine.set_global_enabled(false, false);
    engine.on_key_press(key);
    engine.shutdown();
    engine.set_global_enabled(true, false);

    assert_eq!(
        *events.lock().unwrap(),
        vec!["global:false", "global:true", "toggle:true", "global:false"]
    );
    assert!(!engine
        .global_enabled
        .load(std::sync::atomic::Ordering::SeqCst));
}

#[test]
fn shutdown_stops_same_key_toggle_and_prevents_restart() {
    let key = KeyId::Keyboard(0x51);
    let (engine, rec) = setup(vec![rule("t", BurstMode::Toggle, key, key)]);
    engine.on_key_press(key);
    rec.clear();

    engine.shutdown();
    engine.set_global_enabled(true, false);
    engine.on_key_press(key);
    engine.shutdown();

    assert_eq!(rec.cmds(), vec!["shutdown:g2"]);
    assert!(engine.get_active_ids().is_empty());
}

#[test]
fn dedicated_stop_hotkey_issues_stop_all() {
    let toggle = KeyId::Keyboard(0x51);
    let stop = KeyId::Keyboard(0x71);
    let (engine, rec) = setup(vec![rule(
        "t",
        BurstMode::Toggle,
        toggle,
        KeyId::Keyboard(0x45),
    )]);
    engine.set_hotkeys(qzh_profile::profile::Hotkeys {
        global_toggle: Some(toggle),
        global_stop: Some(stop),
        ..Default::default()
    });

    engine.on_key_press(toggle); // toggle 开
    rec.clear();
    engine.on_key_press(stop); // 专用停止键 → 关全局 → stop_all

    assert_eq!(rec.cmds(), vec!["stopall:g2"]);
}
