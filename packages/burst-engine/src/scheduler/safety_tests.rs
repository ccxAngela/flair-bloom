// 发送替身保留模拟的按下状态，用来区分“账本已清空”和“目标键确实已释放”。
use super::*;
use qzh_profile::{BurstMode, OutputStep};

#[derive(Default)]
struct ReleaseFailure {
    fail_up: AtomicBool,
    physical_down: Mutex<HashSet<KeyId>>,
    events: Mutex<Vec<InputEvent>>,
    duplicate_downs: AtomicU64,
}

impl EventDispatcher for ReleaseFailure {
    fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
        self.events.lock().unwrap().extend_from_slice(events);
        events
            .iter()
            .map(|event| {
                if event.is_up && self.fail_up.load(Ordering::SeqCst) {
                    return DispatchResult::Failed;
                }
                let mut down = self.physical_down.lock().unwrap();
                if event.is_up {
                    down.remove(&event.key);
                } else if !down.insert(event.key) {
                    self.duplicate_downs.fetch_add(1, Ordering::SeqCst);
                }
                DispatchResult::Sent
            })
            .collect()
    }
}

fn worker(dispatcher: Arc<dyn EventDispatcher>) -> SchedulerWorker {
    SchedulerWorker {
        rules: HashMap::new(),
        target_holds: HashMap::new(),
        current_generation: 0,
        dispatcher,
        stats: None,
        simulated_keys: Default::default(),
        min_interval_ms: 10,
        tap_seq: 0,
        progress: Default::default(),
        clock: |now| now,
        cleanup_at: None,
    }
}

fn start(worker: &mut SchedulerWorker, id: &str) {
    start_key(worker, id, KeyId::Keyboard(69));
}

fn start_key(worker: &mut SchedulerWorker, id: &str, key: KeyId) {
    let rule = Arc::new(BurstRule {
        id: id.into(),
        enabled: true,
        trigger_key: KeyId::Keyboard(81),
        mode: BurstMode::Toggle,
        stop_key: None,
        group: None,
        steps: vec![OutputStep {
            key,
            interval_ms: 50,
        }],
    });
    worker.insert_scheduled(Arc::from(id), rule, 0, false, 1);
}

#[test]
fn failed_global_cleanup_must_keep_held_key_for_retry_after_channel_recovers() {
    let dispatcher = Arc::new(ReleaseFailure::default());
    let mut worker = worker(dispatcher.clone());
    start(&mut worker, "rule");
    worker.process_due(Instant::now() + Duration::from_millis(1));
    dispatcher.fail_up.store(true, Ordering::SeqCst);
    worker.cleanup_all();
    assert!(dispatcher
        .physical_down
        .lock()
        .unwrap()
        .contains(&KeyId::Keyboard(69)));
    dispatcher.fail_up.store(false, Ordering::SeqCst);
    worker.cleanup_all();
    assert!(
        dispatcher.physical_down.lock().unwrap().is_empty(),
        "释放失败后账本已清空，通道恢复再停止也不补发 up"
    );
}

#[test]
fn ordinary_release_failure_must_not_deadlock_and_must_keep_the_failed_key_tracked() {
    let dispatcher = Arc::new(ReleaseFailure::default());
    let mut worker = worker(dispatcher.clone());
    start(&mut worker, "rule");
    let now = Instant::now() + Duration::from_millis(1);
    worker.process_due(now);
    dispatcher.fail_up.store(true, Ordering::SeqCst);
    worker.process_due(now + Duration::from_millis(20));
    assert!(!worker.rules.contains_key("rule"));
    assert!(worker
        .simulated_keys
        .lock()
        .unwrap()
        .contains_key(&KeyId::Keyboard(69)));
}

#[test]
fn unreleased_key_must_not_be_reused_as_a_successful_new_tap_by_another_queue() {
    let dispatcher = Arc::new(ReleaseFailure::default());
    let mut worker = worker(dispatcher.clone());
    start(&mut worker, "first");
    let now = Instant::now() + Duration::from_millis(1);
    worker.process_due(now);
    dispatcher.fail_up.store(true, Ordering::SeqCst);
    worker.process_due(now + Duration::from_millis(20));
    assert!(!worker.rules.contains_key("first"));
    assert!(dispatcher
        .physical_down
        .lock()
        .unwrap()
        .contains(&KeyId::Keyboard(69)));
    start(&mut worker, "second");
    start_key(&mut worker, "healthy", KeyId::Keyboard(82));
    worker.process_due(now + Duration::from_millis(21));
    assert_eq!(
        dispatcher.duplicate_downs.load(Ordering::SeqCst),
        0,
        "上一队列释放失败，同键队列仍注入新的 down 并视为成功点按"
    );
}

struct PanicDispatcher;
impl EventDispatcher for PanicDispatcher {
    fn dispatch(&self, _: &[InputEvent]) -> Vec<DispatchResult> {
        panic!("模拟发送线程异常");
    }
}

#[test]
fn scheduler_panic_must_not_leave_or_accept_healthy_running_progress() {
    let keys = SimulatedKeys::default();
    let handle = Arc::new(SchedulerHandle::start_with_dispatcher(
        Arc::new(PanicDispatcher),
        None,
        keys.clone(),
    ));
    let engine = crate::BurstEngine::from_parts(handle.clone(), keys);
    let rule = Arc::new(BurstRule {
        id: "panic".into(),
        enabled: true,
        trigger_key: KeyId::Keyboard(81),
        mode: BurstMode::Toggle,
        stop_key: None,
        group: None,
        steps: vec![OutputStep {
            key: KeyId::Keyboard(69),
            interval_ms: 50,
        }],
    });
    engine.set_rules(vec![(*rule).clone()]);
    engine.set_global_enabled(true, false);
    engine.on_key_press(KeyId::Keyboard(81));
    let deadline = Instant::now() + Duration::from_secs(1);
    while !handle.join.lock().unwrap().as_ref().unwrap().is_finished() {
        assert!(Instant::now() < deadline);
        std::thread::yield_now();
    }
    let states = engine.get_rule_states();
    assert!(states.running.is_empty());
    assert!(states.error.is_some());
    assert!(!engine.global_enabled.load(Ordering::SeqCst));
    engine.set_global_enabled(true, false);
    assert!(!engine.global_enabled.load(Ordering::SeqCst));
    assert!(engine.preview_output(rule.steps.clone()).is_err());
    assert!(engine.start_recording(KeyId::Keyboard(123)).is_err());
    handle.start_rule(rule, 1);
    assert!(
        handle
            .progress()
            .iter()
            .all(|progress| progress.error.is_some()),
        "调度线程已退出，仍报告健康运行并接受新启动：{:?}",
        handle.progress()
    );
}

#[test]
fn cleanup_retries_without_active_rules_and_stops_timer_after_recovery() {
    let dispatcher = Arc::new(ReleaseFailure::default());
    let mut worker = worker(dispatcher.clone());
    start(&mut worker, "rule");
    worker.process_due(Instant::now() + Duration::from_millis(1));
    dispatcher.fail_up.store(true, Ordering::SeqCst);
    let (tx, rx) = mpsc::channel();
    worker.handle_command(SchedulerCommand::StopAll {
        generation: 1,
        ack: Some(tx),
    });
    assert!(!rx.recv().unwrap());
    assert!(worker.rules.is_empty());
    let first_retry = worker.next_deadline().unwrap();
    worker.process_due(first_retry);
    let next_retry = worker.next_deadline().unwrap();
    assert!(next_retry > first_retry);
    dispatcher.fail_up.store(false, Ordering::SeqCst);
    worker.process_due(next_retry);
    assert!(dispatcher.physical_down.lock().unwrap().is_empty());
    assert!(worker.simulated_keys.lock().unwrap().is_empty());
    assert_eq!(worker.next_deadline(), None);
}

#[test]
fn quarantined_queue_keeps_its_step_and_continues_after_release_recovers() {
    let dispatcher = Arc::new(ReleaseFailure::default());
    let mut worker = worker(dispatcher.clone());
    start(&mut worker, "first");
    let now = Instant::now() + Duration::from_millis(1);
    worker.process_due(now);
    dispatcher.fail_up.store(true, Ordering::SeqCst);
    worker.process_due(now + Duration::from_millis(20));
    start(&mut worker, "second");
    start_key(&mut worker, "healthy", KeyId::Keyboard(82));
    worker.process_due(now + Duration::from_millis(21));
    assert_eq!(
        worker.rules["healthy"].queue.held_key(),
        Some(KeyId::Keyboard(82))
    );
    assert_eq!(worker.rules["second"].queue.held_key(), None);
    assert_eq!(worker.rules["second"].queue.completed_cycles(), 0);
    dispatcher.fail_up.store(false, Ordering::SeqCst);
    worker.process_due(now + Duration::from_millis(80));
    assert_eq!(
        worker.rules["second"].queue.held_key(),
        Some(KeyId::Keyboard(69))
    );
    assert_eq!(dispatcher.duplicate_downs.load(Ordering::SeqCst), 0);
}

#[test]
fn fallback_release_keeps_failed_and_unknown_results_until_success() {
    let keys = SimulatedKeys::default();
    keys.lock().unwrap().insert(KeyId::Keyboard(69), 1);
    for result in [Some(DispatchResult::Failed), None] {
        assert!(!release_simulated_keys_using(&keys, |_| result
            .into_iter()
            .collect()));
        assert!(!keys.lock().unwrap().is_empty());
    }
    assert!(!release_simulated_keys_using(&keys, |_| panic!(
        "模拟释放异常"
    )));
    assert!(!keys.lock().unwrap().is_empty());
    assert!(release_simulated_keys_using(&keys, |events| {
        vec![DispatchResult::Sent; events.len()]
    }));
    assert!(keys.lock().unwrap().is_empty());
}

#[test]
fn fallback_and_new_down_are_serialized_without_losing_the_new_down_record() {
    let keys = SimulatedKeys::default();
    keys.lock().unwrap().insert(KeyId::Keyboard(69), 1);
    let dispatcher = Arc::new(ReleaseFailure::default());
    dispatcher
        .physical_down
        .lock()
        .unwrap()
        .insert(KeyId::Keyboard(69));
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let releasing = {
        let keys = keys.clone();
        let dispatcher = dispatcher.clone();
        std::thread::spawn(move || {
            release_simulated_keys_using(&keys, |events| {
                entered_tx.send(()).unwrap();
                resume_rx.recv_timeout(Duration::from_secs(2)).unwrap();
                dispatcher.dispatch(events)
            })
        })
    };
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    let sending = {
        let keys = keys.clone();
        let dispatcher = dispatcher.clone();
        std::thread::spawn(move || {
            dispatch_tracked(
                dispatcher.as_ref(),
                &keys,
                &[InputEvent::down(KeyId::Keyboard(69))],
            )
        })
    };
    resume_tx.send(()).unwrap();
    assert!(releasing.join().unwrap());
    assert_eq!(sending.join().unwrap(), vec![DispatchResult::Sent]);
    assert!(keys.lock().unwrap().contains_key(&KeyId::Keyboard(69)));
    assert_eq!(dispatcher.duplicate_downs.load(Ordering::SeqCst), 0);
}

struct Results(Vec<DispatchResult>);
impl EventDispatcher for Results {
    fn dispatch(&self, _: &[InputEvent]) -> Vec<DispatchResult> {
        self.0.clone()
    }
}

#[test]
fn partial_and_noop_results_never_erase_an_unconfirmed_down() {
    use DispatchResult::{Failed, Noop, Sent};
    for (results, remains_down) in [
        (vec![Sent, Failed], true),
        (vec![Failed, Sent], false),
        (vec![Sent], true),
        (vec![], true),
        (vec![Failed], false),
        (vec![Sent, Noop], true),
        (vec![Noop, Noop], false),
    ] {
        let keys = SimulatedKeys::default();
        dispatch_tracked(
            &Results(results),
            &keys,
            &[
                InputEvent::down(KeyId::Keyboard(69)),
                InputEvent::up(KeyId::Keyboard(69)),
            ],
        );
        assert_eq!(
            keys.lock().unwrap().contains_key(&KeyId::Keyboard(69)),
            remains_down
        );
    }
}

#[test]
fn noop_is_not_a_successful_keyboard_press() {
    let mut worker = worker(Arc::new(Results(vec![DispatchResult::Noop])));
    start(&mut worker, "noop");
    worker.process_due(Instant::now() + Duration::from_millis(1));
    assert!(!worker.rules.contains_key("noop"));
    assert!(worker.simulated_keys.lock().unwrap().is_empty());
}

#[test]
fn panic_before_send_returns_keeps_the_possible_down_tracked() {
    let keys = SimulatedKeys::default();
    assert!(std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        dispatch_tracked(
            &PanicDispatcher,
            &keys,
            &[InputEvent::down(KeyId::Keyboard(69))],
        );
    }))
    .is_err());
    assert!(keys
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains_key(&KeyId::Keyboard(69)));
    assert!(release_simulated_keys_using(&keys, |events| {
        vec![DispatchResult::Sent; events.len()]
    }));
}
