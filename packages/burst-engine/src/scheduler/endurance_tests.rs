use super::*;
use qzh_profile::{BurstMode, OutputStep};

const PROFILE_KEYS: [u32; 8] = [70, 69, 52, 53, 67, 88, 50, 51];

#[derive(Default)]
struct KeyCounts {
    down: u64,
    up: u64,
    held: bool,
}

struct CountingDispatcher {
    keys: Mutex<HashMap<KeyId, KeyCounts>>,
}

impl CountingDispatcher {
    fn new(keys: &[u32]) -> Self {
        Self {
            keys: Mutex::new(
                keys.iter()
                    .map(|vk| (KeyId::Keyboard(*vk), KeyCounts::default()))
                    .collect(),
            ),
        }
    }

    fn total_downs(&self) -> u64 {
        self.keys.lock().unwrap().values().map(|key| key.down).sum()
    }

    fn assert_released(&self) {
        for (key, counts) in self.keys.lock().unwrap().iter() {
            assert!(!counts.held, "停止后仍有按键按住：{key:?}");
            assert_eq!(counts.down, counts.up, "按下和释放不配平：{key:?}");
        }
    }
}

impl EventDispatcher for CountingDispatcher {
    fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
        let mut keys = self.keys.lock().unwrap();
        for event in events {
            let counts = keys.get_mut(&event.key).expect("输出了配置之外的按键");
            if event.is_up {
                assert!(counts.held, "没有按下却释放：{:?}", event.key);
                counts.up += 1;
            } else {
                assert!(!counts.held, "未释放就再次按下：{:?}", event.key);
                counts.down += 1;
            }
            counts.held = !event.is_up;
        }
        vec![DispatchResult::Sent; events.len()]
    }
}

fn worker(keys: &[u32], base: Instant) -> (SchedulerWorker, Arc<CountingDispatcher>) {
    let dispatcher = Arc::new(CountingDispatcher::new(keys));
    let mut worker = SchedulerWorker {
        rules: HashMap::new(),
        target_holds: HashMap::new(),
        current_generation: 0,
        dispatcher: dispatcher.clone(),
        stats: None,
        simulated_keys: Default::default(),
        min_interval_ms: MIN_EFFECTIVE_INTERVAL_MS,
        tap_seq: 0,
        progress: Default::default(),
        clock: |now| now,
        cleanup_at: None,
    };
    for vk in keys {
        let id = format!("hold-{vk}");
        let rule = Arc::new(BurstRule {
            id: id.clone(),
            enabled: true,
            trigger_key: KeyId::Keyboard(*vk),
            mode: BurstMode::Hold,
            stop_key: None,
            group: None,
            steps: vec![OutputStep {
                key: KeyId::Keyboard(*vk),
                interval_ms: 10,
            }],
        });
        worker.progress.lock().unwrap().insert(
            id.clone(),
            RuleProgress {
                rule_id: id.clone(),
                step_index: 0,
                step_count: 1,
                completed_cycles: 0,
                paused: false,
                error: None,
                run_id: 1,
                lateness_ms: 0,
                max_lateness_ms: 0,
            },
        );
        let queue = OutputQueue::new(rule.steps.clone(), base).unwrap();
        worker.insert_scheduled(Arc::from(id.as_str()), rule, 0, false, 1);
        let scheduled = worker.rules.get_mut(id.as_str()).unwrap();
        scheduled.queue = queue;
        scheduled.throttle_at = base;
    }
    (worker, dispatcher)
}

fn assert_bounded(worker: &SchedulerWorker, count: usize) {
    let capacity_limit = (count * 4).max(3);
    assert_eq!(worker.rules.len(), count);
    assert!(worker.rules.capacity() <= capacity_limit);
    assert_eq!(worker.progress.lock().unwrap().len(), count);
    assert!(worker.progress.lock().unwrap().capacity() <= capacity_limit);
    assert!(worker.target_holds.len() <= count);
    assert!(worker.target_holds.capacity() <= capacity_limit);
    assert!(worker.target_holds.values().all(|owners| owners.len() == 1));
    assert!(worker.simulated_keys.lock().unwrap().len() <= count);
    assert!(worker.simulated_keys.lock().unwrap().capacity() <= capacity_limit);
    assert!(worker.cleanup_at.is_none());
    for progress in worker.progress.lock().unwrap().values() {
        assert!(progress.error.is_none());
        assert_eq!(progress.step_index, 0);
        assert_eq!(progress.run_id, 1);
    }
}

fn drive_until(worker: &mut SchedulerWorker, end: Instant) {
    let count = worker.rules.len();
    let mut ticks = 0u64;
    let mut previous = None;
    loop {
        let next = worker.next_deadline().unwrap();
        if next > end {
            break;
        }
        if let Some(previous) = previous {
            assert!(next > previous, "截止时间没有前进");
        }
        worker.process_due(next);
        if ticks.is_multiple_of(4096) {
            assert_bounded(worker, count);
        }
        previous = Some(next);
        ticks += 1;
    }
    assert_bounded(worker, count);
}

fn stop_and_assert_released(worker: &mut SchedulerWorker, dispatcher: &CountingDispatcher) {
    let (tx, rx) = mpsc::channel();
    worker.handle_command(SchedulerCommand::StopAll {
        generation: 1,
        ack: Some(tx),
    });
    assert_eq!(rx.try_recv(), Ok(true));
    assert!(worker.rules.is_empty());
    assert!(worker.target_holds.is_empty());
    assert!(worker.simulated_keys.lock().unwrap().is_empty());
    assert!(worker.next_deadline().is_none());
    dispatcher.assert_released();
}

fn continuous_hold(duration: Duration) {
    for keys in [&PROFILE_KEYS[..1], PROFILE_KEYS.as_slice()] {
        let base = Instant::now();
        let (mut worker, dispatcher) = worker(keys, base);
        drive_until(&mut worker, base + duration);
        let period_ms = u128::from(MIN_EFFECTIVE_INTERVAL_MS) * keys.len() as u128;
        let expected_per_key = duration.as_millis() / period_ms + 1;
        assert_eq!(
            u128::from(dispatcher.total_downs()),
            expected_per_key * keys.len() as u128,
            "连续运行时丢拍、重复发送或节流失效"
        );
        for progress in worker.progress.lock().unwrap().values() {
            assert_eq!(u128::from(progress.completed_cycles), expected_per_key - 1);
            assert!(u128::from(progress.max_lateness_ms) <= period_ms - 10);
        }
        stop_and_assert_released(&mut worker, &dispatcher);
    }
}

#[test]
fn continuous_hold_keeps_state_bounded_and_stops_mid_press() {
    continuous_hold(Duration::from_secs(60));
}

#[test]
#[ignore = "八小时虚拟时钟耐久验证，显式按需运行；不发送真实按键"]
fn eight_hours_of_continuous_hold_keeps_state_bounded_and_stops_mid_press() {
    continuous_hold(Duration::from_secs(8 * 60 * 60));
}

#[test]
fn long_elapsed_time_does_not_wrap_or_replay_missed_taps() {
    for milliseconds in [
        i32::MAX as u64 - 1,
        i32::MAX as u64 + 1,
        u64::from(u32::MAX) - 1,
        u64::from(u32::MAX) + 1,
        365 * 24 * 60 * 60 * 1000,
    ] {
        let base = Instant::now();
        let (mut worker, dispatcher) = worker(&PROFILE_KEYS, base);
        worker.process_due(base);
        let late = base + Duration::from_millis(milliseconds);
        worker.process_due(late);
        assert_eq!(dispatcher.total_downs(), PROFILE_KEYS.len() as u64);
        assert_eq!(
            worker.next_deadline(),
            Some(late + Duration::from_millis(1))
        );
        assert!(worker.simulated_keys.lock().unwrap().is_empty());
        drive_until(&mut worker, late + Duration::from_millis(1000));
        assert_bounded(&worker, PROFILE_KEYS.len());
        assert!(dispatcher.total_downs() < 200, "迟到后追赶补发了积压点按");
        stop_and_assert_released(&mut worker, &dispatcher);
    }
}
