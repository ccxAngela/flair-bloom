use super::*;
use qzh_profile::{BurstMode, OutputStep};

struct FailOnce {
    key: KeyId,
    is_up: bool,
    failed: AtomicBool,
    events: Mutex<Vec<InputEvent>>,
}

impl EventDispatcher for FailOnce {
    fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
        self.events.lock().unwrap().extend_from_slice(events);
        events
            .iter()
            .map(|event| {
                if event.key == self.key
                    && event.is_up == self.is_up
                    && !self.failed.swap(true, Ordering::SeqCst)
                {
                    DispatchResult::Failed
                } else {
                    DispatchResult::Sent
                }
            })
            .collect()
    }
}

fn worker(is_up: bool) -> SchedulerWorker {
    SchedulerWorker {
        rules: HashMap::new(),
        target_holds: HashMap::new(),
        current_generation: 0,
        dispatcher: Arc::new(FailOnce {
            key: KeyId::Keyboard(69),
            is_up,
            failed: AtomicBool::new(false),
            events: Mutex::new(Vec::new()),
        }),
        stats: None,
        simulated_keys: Default::default(),
        min_interval_ms: 1,
        tap_seq: 0,
        progress: Default::default(),
        clock: |now| now,
        cleanup_at: None,
    }
}

fn start(worker: &mut SchedulerWorker, id: &str, keys: &[u32]) {
    let rule = Arc::new(BurstRule {
        id: id.into(),
        enabled: true,
        trigger_key: KeyId::Keyboard(81),
        mode: BurstMode::Toggle,
        stop_key: None,
        group: None,
        steps: keys
            .iter()
            .map(|key| OutputStep {
                key: KeyId::Keyboard(*key),
                interval_ms: 50,
            })
            .collect(),
    });
    worker.progress.lock().unwrap().insert(
        id.into(),
        RuleProgress {
            rule_id: id.into(),
            step_index: 0,
            step_count: keys.len(),
            completed_cycles: 0,
            paused: false,
            error: None,
            run_id: 1,
            lateness_ms: 0,
            max_lateness_ms: 0,
        },
    );
    worker.insert_scheduled(Arc::from(id), rule, 0, false, 1);
}

#[test]
fn failed_down_stops_sequence_and_does_not_release_another_rules_key() {
    let mut worker = worker(false);
    start(&mut worker, "healthy", &[82]);
    start(&mut worker, "failed", &[69, 84]);
    let now = Instant::now() + Duration::from_millis(1);
    worker.process_due(now);
    assert!(!worker.rules.contains_key("failed"));
    assert!(worker.progress.lock().unwrap()["failed"].error.is_some());
    assert!(worker
        .simulated_keys
        .lock()
        .unwrap()
        .contains_key(&KeyId::Keyboard(82)));
    assert!(!worker
        .simulated_keys
        .lock()
        .unwrap()
        .contains_key(&KeyId::Keyboard(69)));
    worker.process_due(now + Duration::from_secs(1));
    assert!(!worker.target_holds.contains_key(&KeyId::Keyboard(84)));
}

#[test]
fn failed_pause_release_retries_only_owned_key_and_does_not_resume() {
    let mut worker = worker(true);
    start(&mut worker, "healthy", &[82]);
    start(&mut worker, "failed", &[69, 84]);
    let now = Instant::now() + Duration::from_millis(1);
    worker.process_due(now);
    worker.pause_rule("failed", now + Duration::from_millis(5));
    assert!(!worker.rules.contains_key("failed"));
    assert!(worker.progress.lock().unwrap()["failed"].error.is_some());
    let held = worker.simulated_keys.lock().unwrap();
    assert!(held.contains_key(&KeyId::Keyboard(82)));
    assert!(!held.contains_key(&KeyId::Keyboard(69)));
}

#[test]
fn old_run_cannot_publish_failure_over_a_restarted_rule() {
    let mut worker = worker(false);
    start(&mut worker, "rule", &[69]);
    worker
        .progress
        .lock()
        .unwrap()
        .get_mut("rule")
        .unwrap()
        .run_id = 2;
    worker.process_due(Instant::now() + Duration::from_millis(1));
    assert_eq!(worker.progress.lock().unwrap()["rule"].error, None);
    assert_eq!(worker.progress.lock().unwrap()["rule"].run_id, 2);
}

#[test]
fn resume_after_failure_keeps_error_and_does_not_create_a_new_queue() {
    let mut worker = worker(false);
    start(&mut worker, "failed", &[69, 84]);
    worker.process_due(Instant::now() + Duration::from_millis(1));
    let failure = worker.progress.lock().unwrap()["failed"].clone();
    worker.handle_command(SchedulerCommand::Resume {
        rule_id: "failed".into(),
        generation: 0,
    });
    assert!(!worker.rules.contains_key("failed"));
    assert_eq!(worker.progress.lock().unwrap()["failed"], failure);
    assert!(failure.error.is_some());
}
