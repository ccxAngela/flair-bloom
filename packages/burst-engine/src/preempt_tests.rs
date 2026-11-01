//! 组内插队（「长按插队，切换让位；松手恢复」）的确定性测试：同时断言下发给调度器的命令序列
//! 与 `get_rule_states` 的运行 / 暂停分区，对应路线图 `docs/roadmaps/archive/hold-interrupt.md` 0.2 行为表。

use crate::test_support::{rule, setup};
use crate::RuleStates;
use qzh_profile::key_id::{KeyId, MouseButton};
use qzh_profile::profile::{BurstMode, BurstRule};

const KEY_A: KeyId = KeyId::Keyboard(0x31); // 1
const KEY_B: KeyId = KeyId::Keyboard(0x32); // 2
const KEY_H: KeyId = KeyId::Keyboard(0x56); // V
const KEY_H2: KeyId = KeyId::Keyboard(0x43); // C
const KEY_STOP: KeyId = KeyId::Keyboard(0x58); // X

fn grouped(id: &str, mode: BurstMode, trigger: KeyId, target: u32, group: &str) -> BurstRule {
    let mut r = rule(id, mode, trigger, KeyId::Keyboard(target));
    r.group = Some(group.to_string());
    r
}

fn toggle(id: &str, trigger: KeyId, group: &str) -> BurstRule {
    grouped(id, BurstMode::Toggle, trigger, 0x51, group)
}

fn hold(id: &str, trigger: KeyId, group: &str) -> BurstRule {
    grouped(id, BurstMode::Hold, trigger, 0x52, group)
}

fn states(running: &[&str], paused: &[&str]) -> RuleStates {
    RuleStates {
        running: running.iter().map(|s| s.to_string()).collect(),
        paused: paused.iter().map(|s| s.to_string()).collect(),
    }
}

/// 按下并松开（切换键）：切换规则只看按下，松开只为清物理账本。
fn tap(engine: &crate::BurstEngine, key: KeyId) {
    engine.on_key_press(key);
    engine.on_key_release(key);
}

#[test]
fn toggle_start_without_hold_keeps_plain_mutex_replacement() {
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g"), toggle("b", KEY_B, "g")]);

    tap(&engine, KEY_A);
    tap(&engine, KEY_B);

    assert_eq!(rec.cmds(), vec!["start:a:g1", "stop:a:g1", "start:b:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["b"], &[]));
}

#[test]
fn group_hold_press_pauses_running_toggle_then_release_resumes_it() {
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g"), hold("h", KEY_H, "g")]);
    tap(&engine, KEY_A);
    rec.clear();

    engine.on_key_press(KEY_H);
    assert_eq!(rec.take(), vec!["stop:a:g1", "start:h:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["h"], &["a"]));
    // 暂停的规则仍算活跃，界面不会把让位显示成已关闭。
    assert_eq!(engine.get_active_ids(), vec!["a", "h"]);

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h:g1", "start:a:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["a"], &[]));
}

#[test]
fn nested_holds_run_latest_and_hand_back_in_reverse_order() {
    let (engine, rec) = setup(vec![
        toggle("a", KEY_A, "g"),
        hold("h1", KEY_H, "g"),
        hold("h2", KEY_H2, "g"),
    ]);
    tap(&engine, KEY_A);
    rec.clear();

    engine.on_key_press(KEY_H);
    engine.on_key_press(KEY_H2);
    assert_eq!(
        rec.take(),
        vec!["stop:a:g1", "start:h1:g1", "stop:h1:g1", "start:h2:g1"]
    );
    assert_eq!(engine.get_rule_states(), states(&["h2"], &["a", "h1"]));

    engine.on_key_release(KEY_H2);
    assert_eq!(rec.take(), vec!["stop:h2:g1", "start:h1:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["h1"], &["a"]));

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h1:g1", "start:a:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["a"], &[]));
}

#[test]
fn releasing_middle_of_stack_only_pops_it_without_touching_top() {
    let (engine, rec) = setup(vec![
        toggle("a", KEY_A, "g"),
        hold("h1", KEY_H, "g"),
        hold("h2", KEY_H2, "g"),
    ]);
    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);
    engine.on_key_press(KEY_H2);
    rec.clear();

    engine.on_key_release(KEY_H);
    assert!(rec.take().is_empty());
    assert_eq!(engine.get_rule_states(), states(&["h2"], &["a"]));

    engine.on_key_release(KEY_H2);
    assert_eq!(rec.take(), vec!["stop:h2:g1", "start:a:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["a"], &[]));
}

#[test]
fn pressing_paused_toggle_key_removes_it_without_stop_and_it_does_not_resume() {
    // 默认配置下停止键即启动键：暂停期间再按一次切换键就是「停止」。
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g"), hold("h", KEY_H, "g")]);
    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);
    rec.clear();

    tap(&engine, KEY_A);
    assert!(rec.take().is_empty());
    assert_eq!(engine.get_rule_states(), states(&["h"], &[]));

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h:g1"]);
    assert!(engine.get_active_ids().is_empty());
}

#[test]
fn separate_stop_key_on_paused_toggle_removes_it_without_stop() {
    let mut a = toggle("a", KEY_A, "g");
    a.stop_key = Some(KEY_STOP);
    let (engine, rec) = setup(vec![a, hold("h", KEY_H, "g")]);
    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);
    rec.clear();

    tap(&engine, KEY_STOP);
    assert!(rec.take().is_empty());
    assert_eq!(engine.get_rule_states(), states(&["h"], &[]));

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h:g1"]);
}

#[test]
fn toggle_replacement_during_hold_keeps_new_rule_paused_until_release() {
    let (engine, rec) = setup(vec![
        toggle("a", KEY_A, "g"),
        toggle("b", KEY_B, "g"),
        hold("h", KEY_H, "g"),
    ]);
    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);
    rec.clear();

    // a 已暂停不发 stop；按住的 h 不被挤掉，b 只成为恢复目标、不发 start。
    tap(&engine, KEY_B);
    assert!(rec.take().is_empty());
    assert_eq!(engine.get_rule_states(), states(&["h"], &["b"]));

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h:g1", "start:b:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["b"], &[]));
}

#[test]
fn toggle_start_during_hold_with_empty_group_waits_for_release() {
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g"), hold("h", KEY_H, "g")]);
    engine.on_key_press(KEY_H);
    rec.clear();

    tap(&engine, KEY_A);
    assert!(rec.take().is_empty());
    assert_eq!(engine.get_rule_states(), states(&["h"], &["a"]));

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h:g1", "start:a:g1"]);
}

#[test]
fn group_hold_alone_behaves_like_plain_hold() {
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g"), hold("h", KEY_H, "g")]);

    engine.on_key_press(KEY_H);
    assert_eq!(engine.get_rule_states(), states(&["h"], &[]));
    engine.on_key_release(KEY_H);

    assert_eq!(rec.cmds(), vec!["start:h:g1", "stop:h:g1"]);
    assert_eq!(engine.get_rule_states(), RuleStates::default());
}

#[test]
fn ungrouped_hold_runs_alongside_group_toggle() {
    let (engine, rec) = setup(vec![
        toggle("a", KEY_A, "g"),
        rule("h", BurstMode::Hold, KEY_H, KeyId::Keyboard(0x52)),
    ]);
    tap(&engine, KEY_A);
    rec.clear();

    engine.on_key_press(KEY_H);
    assert_eq!(rec.take(), vec!["start:h:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["a", "h"], &[]));

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["a"], &[]));
}

#[test]
fn hold_in_other_group_does_not_preempt() {
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g1"), hold("h", KEY_H, "g2")]);
    tap(&engine, KEY_A);
    rec.clear();

    engine.on_key_press(KEY_H);
    assert_eq!(rec.take(), vec!["start:h:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["a", "h"], &[]));

    // g2 的栈不影响 g1 的切换启动。
    tap(&engine, KEY_A);
    tap(&engine, KEY_A);
    assert_eq!(rec.take(), vec!["stop:a:g1", "start:a:g1"]);

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h:g1"]);
}

#[test]
fn wheel_hold_in_group_taps_without_preempting_toggle() {
    let wheel = KeyId::Mouse(MouseButton::WheelUp);
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g"), hold("w", wheel, "g")]);
    tap(&engine, KEY_A);
    rec.clear();

    tap(&engine, wheel);

    assert_eq!(rec.cmds(), vec!["tap:w:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["a"], &[]));
}

#[test]
fn repeated_hold_down_does_not_push_twice() {
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g"), hold("h", KEY_H, "g")]);
    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);
    engine.on_key_press(KEY_H); // 键盘自动重复
    rec.clear();

    engine.on_key_release(KEY_H);
    assert_eq!(rec.take(), vec!["stop:h:g1", "start:a:g1"]);
    assert_eq!(engine.get_rule_states(), states(&["a"], &[]));
}

fn assert_stack_cleared_by(reset: impl Fn(&crate::BurstEngine)) {
    let (engine, rec) = setup(vec![toggle("a", KEY_A, "g"), hold("h", KEY_H, "g")]);
    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);

    reset(&engine);
    assert_eq!(engine.get_rule_states(), RuleStates::default());
    rec.clear();

    // 复位清了物理账本：松开旧 h 被当作未按下而早返回，不会误出栈或恢复。
    assert!(!engine.on_key_release_event(KEY_H).accepted_physical);
    assert!(rec.take().is_empty());

    // 栈已清空：再按组内切换正常启动，不会被残留栈当成插队中而保持暂停。
    tap(&engine, KEY_A);
    assert_eq!(rec.take(), vec!["start:a:g2"]);
    assert_eq!(engine.get_rule_states(), states(&["a"], &[]));
}

#[test]
fn global_disable_clears_stack_and_pause_state() {
    assert_stack_cleared_by(|engine| {
        engine.set_global_enabled(false, false);
        engine.set_global_enabled(true, false);
    });
}

#[test]
fn backend_switch_clears_stack_and_pause_state() {
    assert_stack_cleared_by(|engine| {
        engine.begin_backend_switch();
        engine.end_backend_switch();
    });
}

#[test]
fn shutdown_clears_stack_and_pause_state() {
    let (engine, _rec) = setup(vec![toggle("a", KEY_A, "g"), hold("h", KEY_H, "g")]);
    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);

    engine.shutdown();

    assert_eq!(engine.get_rule_states(), RuleStates::default());
}

#[test]
fn rule_states_are_partitioned_and_sorted() {
    let (engine, _rec) = setup(vec![
        toggle("z-toggle", KEY_A, "g"),
        hold("m-hold", KEY_H, "g"),
        hold("a-hold", KEY_H2, "g"),
        rule("b-free", BurstMode::Hold, KEY_B, KeyId::Keyboard(0x53)),
    ]);
    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);
    engine.on_key_press(KEY_H2);
    engine.on_key_press(KEY_B);

    assert_eq!(
        engine.get_rule_states(),
        states(&["a-hold", "b-free"], &["m-hold", "z-toggle"])
    );
    assert_eq!(
        engine.get_active_ids(),
        vec!["a-hold", "b-free", "m-hold", "z-toggle"]
    );
}

/// 卡在第一次「停 h」上的调度器替身：给另一线程留出插进来的窗口，看命令顺序会不会与状态变更相反。
struct GatedScheduler {
    cmds: std::sync::Mutex<Vec<String>>,
    entered: std::sync::Mutex<Option<std::sync::mpsc::Sender<()>>>,
    other_done: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
}

impl crate::scheduler::Scheduler for GatedScheduler {
    fn start_rule(&self, rule: std::sync::Arc<BurstRule>, _generation: u64) {
        self.cmds.lock().unwrap().push(format!("start:{}", rule.id));
    }
    fn tap_once(&self, _rule: std::sync::Arc<BurstRule>, _generation: u64) {}
    fn stop_rule(&self, rule_id: String, _generation: u64) {
        if rule_id == "h" {
            if let Some(entered) = self.entered.lock().unwrap().take() {
                let _ = entered.send(());
                // 命令在锁外发时，另一线程此刻能跑完整个停止流程；锁内发时它卡在 runtime 锁上，只能超时。
                let _ = self
                    .other_done
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_millis(300));
            }
        }
        self.cmds.lock().unwrap().push(format!("stop:{rule_id}"));
    }
    fn stop_all_async(&self, _generation: u64) {}
    fn stop_all_blocking(&self, _generation: u64) -> bool {
        true
    }
    fn shutdown_blocking(&self, _generation: u64) -> bool {
        true
    }
    fn hp_degraded(&self) -> bool {
        false
    }
}

#[test]
fn concurrent_hold_release_and_toggle_stop_keep_scheduler_in_sync() {
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let sched = std::sync::Arc::new(GatedScheduler {
        cmds: Default::default(),
        entered: std::sync::Mutex::new(Some(entered_tx)),
        other_done: std::sync::Mutex::new(done_rx),
    });
    let engine = std::sync::Arc::new(crate::BurstEngine::new_with_scheduler(sched.clone()));
    let mut t = toggle("t", KEY_A, "g");
    t.stop_key = Some(KEY_STOP);
    engine.set_rules(vec![t, hold("h", KEY_H, "g")]);
    engine.set_global_enabled(true, false);

    tap(&engine, KEY_A);
    engine.on_key_press(KEY_H);
    assert_eq!(engine.get_rule_states(), states(&["h"], &["t"]));

    // 线程 A 松开长按：把 t 恢复为在跑，并发「停 h、启 t」。
    let a = {
        let engine = engine.clone();
        std::thread::spawn(move || engine.on_key_release(KEY_H))
    };
    // 线程 B 在 A 发命令的窗口里按 t 的停止键。
    entered_rx.recv().unwrap();
    let b = {
        let engine = engine.clone();
        std::thread::spawn(move || {
            tap(&engine, KEY_STOP);
            let _ = done_tx.send(());
        })
    };
    a.join().unwrap();
    b.join().unwrap();

    assert_eq!(engine.get_rule_states(), states(&[], &[]));
    let last_t = sched
        .cmds
        .lock()
        .unwrap()
        .iter()
        .rev()
        .find(|c| c.ends_with(":t"))
        .cloned();
    assert_eq!(
        last_t.as_deref(),
        Some("stop:t"),
        "引擎认为 t 已停，调度器收到的最后一条也必须是停"
    );
}
