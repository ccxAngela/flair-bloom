//! 调度器确定性模拟测试 harness（参考 kanata 的 `simulated_input`）。
//!
//! 思路：连发的所有时序逻辑都在 [`SchedulerWorker::process_due`] 里，而它**把当前时刻
//! `now` 作为入参**——这意味着只要喂合成的 `now`（虚拟时钟），就能脱离真实 wall-clock、
//! 真线程、真 waitable timer，得到**完全确定性**的注入事件序列。配一个只记录事件的
//! [`RecordingDispatcher`]（替代真实 `win_input` 注入），即可对「按下/抬起/时序」做
//! golden 断言。
//!
//! 用法（脚本 DSL，空白分隔）：
//! - `start:<id>` 启动某规则      - `stop:<id>` 停止某规则
//! - `stopall`    停止全部        - `t:<ms>`    推进虚拟时间 N 毫秒（逐毫秒 tick）
//!
//! 输出：每个注入事件渲染成 `<毫秒>:dn:<键>` / `<毫秒>:up:<键>`，按 (时刻, 抬起, 键)
//! 稳定排序后空格连接——同一毫秒内跨规则的事件顺序本就不确定，排序消除 HashMap 抖动，
//! 同键的 down 永远排在 up 前。键渲染：键盘 `K<十六进制VK>`，鼠标 `M<按钮名>`。
//!
//! 新增场景测试只需写一行脚本 + 期望串，见文件末尾各 `#[test]`。

use super::{EventDispatcher, SchedulerCommand, SchedulerWorker};
use qzh_profile::key_id::{KeyId, MouseButton};
use qzh_profile::profile::{BurstMode, BurstRule};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use win_input::{DispatchResult, InputEvent};

/// 只把注入事件记进缓冲、不碰 OS 的测试 dispatcher。
struct RecordingDispatcher {
    events: Mutex<Vec<InputEvent>>,
}

impl EventDispatcher for RecordingDispatcher {
    fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
        self.events
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .extend_from_slice(events);
        vec![DispatchResult::Sent; events.len()]
    }
}

impl RecordingDispatcher {
    fn new() -> Self {
        Self {
            events: Mutex::new(Vec::new()),
        }
    }

    fn drain(&self) -> Vec<InputEvent> {
        std::mem::take(&mut self.events.lock().unwrap_or_else(|e| e.into_inner()))
    }
}

/// 虚拟时钟模拟器：直接驱动 [`SchedulerWorker`]，不经 `worker_loop` 的真线程/真 timer。
struct Sim {
    worker: SchedulerWorker,
    recorder: Arc<RecordingDispatcher>,
    rules: HashMap<String, Arc<BurstRule>>,
    base: Instant,
    now_ms: u64,
    timeline: Vec<(u64, InputEvent)>,
}

impl Sim {
    fn new(rules: Vec<BurstRule>) -> Self {
        let recorder = Arc::new(RecordingDispatcher::new());
        let dispatcher: Arc<dyn EventDispatcher> = recorder.clone();
        let worker = SchedulerWorker {
            activity: Arc::new(crate::RuleActivity::channel().0),
            rules: HashMap::new(),
            target_holds: HashMap::new(),
            current_generation: 0,
            dispatcher,
            stats: None,
            simulated_keys: Arc::new(Mutex::new(HashMap::new())),
            // 纯时序 golden 断言不限速；下限本身的行为单独由 min_interval_floor_* 测试覆盖。
            min_interval_ms: 1,
            tap_seq: 0,
            progress: Default::default(),
            clock: |now| now,
            cleanup_at: None,
        };
        let rules = rules
            .into_iter()
            .map(|r| (r.id.clone(), Arc::new(r)))
            .collect();
        Self {
            worker,
            recorder,
            rules,
            base: Instant::now(),
            now_ms: 0,
            timeline: Vec::new(),
        }
    }

    /// 把录制 dispatcher 里积累的事件按当前虚拟时刻打点收进时间线。
    fn drain_now(&mut self) {
        for e in self.recorder.drain() {
            self.timeline.push((self.now_ms, e));
        }
    }

    fn start(&mut self, id: &str) {
        let rule = self.rules.get(id).expect("未知规则 id").clone();
        self.worker.handle_command(SchedulerCommand::Start {
            rule,
            generation: 0,
            run_id: 1,
            once: false,
        });
        // Start 内部用真实 `Instant::now()` 设了 next_at；覆盖成当前虚拟时刻，锚定虚拟时钟。
        if let Some(sr) = self.worker.rules.get_mut(id) {
            let now = self.base + Duration::from_millis(self.now_ms);
            sr.queue =
                crate::output_queue::OutputQueue::new(self.rules[id].steps.clone(), now).unwrap();
            sr.throttle_at = now;
        }
        self.drain_now();
    }

    fn stop(&mut self, id: &str) {
        self.worker.handle_command(SchedulerCommand::Stop {
            rule_id: id.to_string(),
            generation: 0,
            ack: None,
        });
        self.drain_now();
    }

    fn stop_all(&mut self) {
        self.worker.handle_command(SchedulerCommand::StopAll {
            generation: 0,
            ack: None,
        });
        self.drain_now();
    }

    /// 设定注入周期基础下限（生产取 MIN_EFFECTIVE_INTERVAL_MS，纯时序测试默认 1ms 不限速）。
    fn set_min_interval(&mut self, ms: u32) {
        self.worker.min_interval_ms = ms;
    }

    /// 逐毫秒推进虚拟时间，每毫秒处理一次到期事件——确定性的关键。
    fn advance(&mut self, ms: u64) {
        for _ in 0..ms {
            self.now_ms += 1;
            let now = self.base + Duration::from_millis(self.now_ms);
            self.worker.process_due(now);
            self.drain_now();
        }
    }

    fn timeline_string(&self) -> String {
        let mut items: Vec<(u64, bool, String)> = self
            .timeline
            .iter()
            .map(|(ms, e)| (*ms, e.is_up, fmt_key(e.key)))
            .collect();
        // 同毫秒内跨规则顺序本就不确定，稳定排序消抖；同键 down(false) 排在 up(true) 前。
        items.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        items
            .iter()
            .map(|(ms, up, k)| format!("{ms}:{}:{k}", if *up { "up" } else { "dn" }))
            .collect::<Vec<_>>()
            .join(" ")
    }
}

fn fmt_key(k: KeyId) -> String {
    match k {
        KeyId::Keyboard(vk) => format!("K{vk:02X}"),
        KeyId::Mouse(b) => format!("M{b:?}"),
    }
}

/// 跑一段脚本，返回 golden 时间线串。
fn simulate(rules: Vec<BurstRule>, script: &str) -> String {
    let mut sim = Sim::new(rules);
    for tok in script.split_whitespace() {
        if tok == "stopall" {
            sim.stop_all();
            continue;
        }
        match tok.split_once(':') {
            Some(("start", id)) => sim.start(id),
            Some(("stop", id)) => sim.stop(id),
            Some(("t", ms)) => sim.advance(ms.parse().expect("t: 后需为毫秒数")),
            _ => panic!("非法脚本 token: {tok:?}"),
        }
    }
    sim.timeline_string()
}

const E: KeyId = KeyId::Keyboard(0x45);
const R_KEY: KeyId = KeyId::Keyboard(0x52);

fn hold(id: &str, target: KeyId, interval_ms: u32) -> BurstRule {
    rule(id, target, interval_ms, None)
}

fn rule(id: &str, target: KeyId, interval_ms: u32, group: Option<String>) -> BurstRule {
    BurstRule {
        id: id.to_string(),
        enabled: true,
        // 调度器层不看 trigger/mode/stop（那是引擎层的事），固定占位即可。
        trigger_key: KeyId::Keyboard(0x51),
        steps: vec![qzh_profile::profile::OutputStep {
            key: target,
            interval_ms,
        }],
        mode: BurstMode::Hold,
        stop_key: None,
        group,
    }
}

// ----- 常规场景 -----

#[test]
fn tap_mode_emits_down_and_up_same_tick_each_ms() {
    // interval=1ms 为极速「点按」档：每毫秒一组 down+up，按下时长为 0。
    let out = simulate(vec![hold("r", E, 1)], "start:r t:5");
    assert_eq!(
        out,
        "1:dn:K45 1:up:K45 2:dn:K45 2:up:K45 3:dn:K45 3:up:K45 4:dn:K45 4:up:K45 5:dn:K45 5:up:K45"
    );
}

#[test]
fn hold_mode_separates_down_and_up_by_hold_duration() {
    // interval=10ms：按下时长 hold_duration(10)=3ms，间隔 7ms，节拍 10ms 一次。
    let out = simulate(vec![hold("r", E, 10)], "start:r t:25");
    assert_eq!(
        out,
        "1:dn:K45 4:up:K45 11:dn:K45 14:up:K45 21:dn:K45 24:up:K45"
    );
}

#[test]
fn cadence_holds_steady_over_time() {
    // 下降沿（dn）应每 interval(10ms) 一次：1, 11, 21, 31 …
    let out = simulate(vec![hold("r", E, 10)], "start:r t:35");
    let downs: Vec<&str> = out
        .split_whitespace()
        .filter(|t| t.contains(":dn:"))
        .collect();
    assert_eq!(
        downs,
        vec!["1:dn:K45", "11:dn:K45", "21:dn:K45", "31:dn:K45"]
    );
}

// ----- 边界场景 -----

#[test]
fn interval_two_still_has_one_ms_hold_not_tap() {
    // 边界：interval=2ms 不是点按档，按下时长被钳到至少 1ms。
    let out = simulate(vec![hold("r", E, 2)], "start:r t:5");
    assert_eq!(out, "1:dn:K45 2:up:K45 3:dn:K45 4:up:K45 5:dn:K45");
}

#[test]
fn stop_releases_a_currently_held_target() {
    // 在「按下中」停止：必须补发 up，避免目标键卡在按下态。
    let out = simulate(vec![hold("r", E, 10)], "start:r t:2 stop:r");
    assert_eq!(out, "1:dn:K45 2:up:K45");
}

#[test]
fn stop_during_rest_phase_emits_no_extra_up() {
    // 在「间隔/抬起后」停止：此刻未按下，不应补发多余 up。
    let out = simulate(vec![hold("r", E, 10)], "start:r t:5 stop:r");
    assert_eq!(out, "1:dn:K45 4:up:K45");
}

#[test]
fn shared_target_emits_single_down() {
    // 两条规则同一目标键：只发一次 down（共享所有权），避免相互打断。
    let out = simulate(
        vec![hold("a", E, 10), hold("b", E, 10)],
        "start:a start:b t:1",
    );
    assert_eq!(out, "1:dn:K45");
}

#[test]
fn shared_target_releases_once_on_stop_all() {
    // 同目标键被两规则共享：stopall 只补发一次 up（最后一个 owner 释放）。
    let out = simulate(
        vec![hold("a", E, 10), hold("b", E, 10)],
        "start:a start:b t:1 stopall",
    );
    assert_eq!(out, "1:dn:K45 1:up:K45");
}

#[test]
fn stop_all_releases_every_distinct_held_target() {
    // 不同目标键各自被按住：stopall 必须把每个键都释放。
    let out = simulate(
        vec![hold("a", E, 10), hold("b", R_KEY, 10)],
        "start:a start:b t:1 stopall",
    );
    assert_eq!(out, "1:dn:K45 1:dn:K52 1:up:K45 1:up:K52");
}

#[test]
fn restart_after_stop_resumes_cleanly() {
    // 停止后再启动：状态干净复位，从新的虚拟时刻重新起拍。
    let out = simulate(vec![hold("r", E, 10)], "start:r t:2 stop:r start:r t:2");
    assert_eq!(out, "1:dn:K45 2:up:K45 3:dn:K45");
}

#[test]
fn mouse_target_is_dispatched_like_keyboard() {
    // 鼠标键作为连发目标：与键盘同样走 down/up 序列。
    let out = simulate(
        vec![hold("r", KeyId::Mouse(MouseButton::Left), 1)],
        "start:r t:2",
    );
    assert_eq!(out, "1:dn:MLeft 1:up:MLeft 2:dn:MLeft 2:up:MLeft");
}

#[test]
fn stopping_one_of_two_rules_leaves_the_other_running() {
    // 边界：多规则并存，停止其一（补发其 up）不影响另一条继续连发。
    let out = simulate(
        vec![hold("a", E, 10), hold("b", R_KEY, 10)],
        "start:a start:b t:2 stop:a t:10",
    );
    assert_eq!(out, "1:dn:K45 1:dn:K52 2:up:K45 4:up:K52 11:dn:K52");
}

// ----- 有效注入周期硬下限（防超发冲破管线天花板导致「收不住」；生产基础下限见
// MIN_EFFECTIVE_INTERVAL_MS=10ms，本节用 set_min_interval 注入任意测试值以验证钳位机制）-----

#[test]
fn min_interval_floor_throttles_sub_floor_cadence() {
    // 注入 16ms 测试下限：interval=10ms 被钳成 16ms 拍距，dn 落在 1 / 17 / 33，
    // 但 hold（按下时长=hold_duration(10)=3ms）不受下限影响，保持点按手感（up 紧跟 3ms 后）。
    let mut sim = Sim::new(vec![hold("r", E, 10)]);
    sim.set_min_interval(16);
    sim.start("r");
    sim.advance(40);
    assert_eq!(
        sim.timeline_string(),
        "1:dn:K45 4:up:K45 17:dn:K45 20:up:K45 33:dn:K45 36:up:K45"
    );
}

#[test]
fn interval_above_floor_is_unaffected() {
    // 规则间隔已高于测试下限（20ms > 16ms）：下限不介入，拍距仍为 20ms（dn 落在 1 / 21）。
    let mut sim = Sim::new(vec![hold("r", E, 20)]);
    sim.set_min_interval(16);
    sim.start("r");
    sim.advance(25);
    assert_eq!(sim.timeline_string(), "1:dn:K45 7:up:K45 21:dn:K45");
}

#[test]
fn floor_scales_with_active_rule_count_to_cap_total_rate() {
    // 总并发限速：3 条规则同时连发，每条有效下限 = 基础下限(10) × 3 = 30ms。
    // 取规则 a（K45）的 dn 时刻应为 30ms 拍距（1 / 31 / 61），总 tap 速率仍 ≈ 1000/基础、与规则数无关。
    let t = KeyId::Keyboard(0x54);
    let mut sim = Sim::new(vec![
        hold("a", E, 10),
        hold("b", R_KEY, 10),
        hold("c", t, 10),
    ]);
    sim.set_min_interval(10);
    sim.start("a");
    sim.start("b");
    sim.start("c");
    sim.advance(65);
    let tl = sim.timeline_string();
    let downs: Vec<&str> = tl
        .split_whitespace()
        .filter(|s| s.contains(":dn:K45"))
        .collect();
    assert_eq!(downs, vec!["1:dn:K45", "31:dn:K45", "61:dn:K45"]);
}

fn queue() -> BurstRule {
    let mut r = hold("queue", E, 50);
    r.steps.extend([
        qzh_profile::OutputStep {
            key: E,
            interval_ms: 80,
        },
        qzh_profile::OutputStep {
            key: R_KEY,
            interval_ms: 40,
        },
    ]);
    r
}

#[test]
fn contending_queues_finish_each_step_without_starving_later_rules() {
    let rules = (0..10)
        .map(|index| hold(&format!("r{index:02}"), E, 50))
        .collect();
    let mut sim = Sim::new(rules);
    sim.set_min_interval(10);
    for index in 0..10 {
        sim.start(&format!("r{index:02}"));
    }
    sim.advance(200);
    assert!(sim
        .worker
        .rules
        .values()
        .all(|rule| rule.queue.completed_cycles() >= 1));
    sim.stop_all();
    assert!(sim.worker.simulated_keys.lock().unwrap().is_empty());
}

#[test]
fn queue_preserves_repeated_keys_and_all_intervals_across_cycles() {
    let mut sim = Sim::new(vec![queue()]);
    sim.start("queue");
    sim.advance(370);
    let downs: Vec<_> = sim
        .timeline
        .iter()
        .filter(|(_, event)| !event.is_up)
        .map(|(ms, event)| (*ms, event.key))
        .collect();
    assert_eq!(
        downs,
        vec![
            (1, E),
            (51, E),
            (131, R_KEY),
            (171, E),
            (221, E),
            (301, R_KEY),
            (341, E)
        ]
    );
    assert_eq!(
        sim.timeline.iter().filter(|(_, event)| event.is_up).count(),
        downs.len()
    );
    assert_eq!(sim.worker.rules["queue"].queue.completed_cycles(), 2);
}

#[test]
fn pause_releases_down_and_resumes_remaining_wait_without_replaying_step() {
    for pause_at in [10, 30] {
        let mut sim = Sim::new(vec![queue()]);
        sim.start("queue");
        sim.advance(pause_at);
        sim.worker
            .pause_rule("queue", sim.base + Duration::from_millis(sim.now_ms));
        sim.drain_now();
        assert!(sim.worker.simulated_keys.lock().unwrap().is_empty());
        let remaining = sim.worker.rules["queue"].queue.paused_remaining().unwrap();
        assert_eq!(remaining.as_millis(), 51 - u128::from(pause_at));
        let before = sim.timeline.len();
        sim.advance(1000);
        assert_eq!(sim.timeline.len(), before);
        sim.worker
            .resume_rule("queue", sim.base + Duration::from_millis(sim.now_ms));
        sim.advance(remaining.as_millis() as u64);
        let downs: Vec<_> = sim
            .timeline
            .iter()
            .filter(|(_, event)| !event.is_up)
            .map(|(ms, event)| (*ms, event.key))
            .collect();
        assert_eq!(downs, vec![(1, E), (1051, E)]);
        assert_eq!(sim.worker.rules["queue"].queue.step_index(), 1);
    }
}

#[test]
fn stop_during_long_interval_discards_all_future_steps_and_restart_is_first_step() {
    let mut r = queue();
    r.steps[0].interval_ms = 10000;
    let mut sim = Sim::new(vec![r]);
    sim.start("queue");
    sim.advance(5);
    sim.stop("queue");
    let before = sim.timeline.len();
    sim.advance(20000);
    assert_eq!(sim.timeline.len(), before);
    assert!(sim.worker.simulated_keys.lock().unwrap().is_empty());
    sim.start("queue");
    sim.advance(1);
    assert_eq!(sim.worker.rules["queue"].queue.step_index(), 0);
    assert_eq!(sim.timeline.last().unwrap().1.key, E);
}

#[test]
fn one_shot_queue_finishes_the_whole_sequence_exactly_once() {
    let mut sim = Sim::new(vec![queue()]);
    sim.start("queue");
    sim.worker.rules.get_mut("queue").unwrap().one_shot = true;
    sim.advance(500);
    let downs: Vec<_> = sim
        .timeline
        .iter()
        .filter(|(_, event)| !event.is_up)
        .map(|(_, event)| event.key)
        .collect();
    assert_eq!(downs, vec![E, E, R_KEY]);
    assert!(sim.worker.rules.is_empty());
    assert!(sim.worker.simulated_keys.lock().unwrap().is_empty());
}
