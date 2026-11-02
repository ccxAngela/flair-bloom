use super::*;
use crate::test_support::{rule, setup};
use std::sync::mpsc;
use std::time::Duration;

fn pause_routing(engine: &BurstEngine) -> (mpsc::Receiver<()>, mpsc::Sender<()>) {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    *revive(engine.routing_pause.lock()) = Some(Box::new(move || {
        entered_tx.send(()).unwrap();
        resume_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    }));
    (entered_rx, resume_tx)
}

#[test]
fn stale_routed_input_cannot_cross_rule_update_recording_or_backend_switch() {
    for boundary in [0, 1, 2] {
        for mode in [BurstMode::Hold, BurstMode::Toggle] {
            let trigger = if boundary == 1 {
                KeyId::Mouse(MouseButton::WheelUp)
            } else {
                KeyId::Keyboard(81)
            };
            let (engine, scheduler) =
                setup(vec![rule("before", mode, trigger, KeyId::Keyboard(69))]);
            let engine = Arc::new(engine);
            let (entered, resume) = pause_routing(&engine);
            let pressing = {
                let engine = engine.clone();
                std::thread::spawn(move || engine.on_key_press(trigger))
            };
            entered.recv_timeout(Duration::from_secs(2)).unwrap();
            match boundary {
                0 => engine.set_rules(vec![]),
                1 => {
                    engine.start_recording(KeyId::Keyboard(123)).unwrap();
                }
                _ => {
                    engine.begin_backend_switch().unwrap();
                }
            }
            scheduler.clear();
            resume.send(()).unwrap();
            assert!(!pressing.join().unwrap());
            engine.on_key_release(trigger);
            assert!(scheduler.cmds().is_empty());
            assert!(engine.get_active_ids().is_empty());
        }
    }
}

#[test]
fn stopped_hold_waits_for_release_before_accepting_a_new_press() {
    let trigger = KeyId::Keyboard(81);
    let (engine, scheduler) = setup(vec![rule(
        "held",
        BurstMode::Hold,
        trigger,
        KeyId::Keyboard(69),
    )]);
    engine.on_key_press(trigger);
    engine.cancel_all_loops();
    scheduler.clear();
    assert!(!engine.on_key_press(trigger));
    assert!(scheduler.cmds().is_empty());
    engine.on_key_release(trigger);
    assert!(engine.on_key_press(trigger));
    assert_eq!(scheduler.cmds(), vec!["start:held:g2"]);
}

#[test]
fn shared_stop_autorepeat_never_reenables_output_after_recording_cancel() {
    let (engine, _) = setup(vec![]);
    engine.set_global_enabled(false, false);
    let key = KeyId::Keyboard(122);
    engine.set_hotkeys(Hotkeys {
        global_toggle: Some(key),
        ..Default::default()
    });
    engine.start_recording(KeyId::Keyboard(123)).unwrap();
    engine.on_key_press(key);
    assert!(!engine.get_recording().recording);
    engine.on_key_press(key);
    assert!(!engine.global_enabled.load(Ordering::SeqCst));
    engine.on_key_release(key);
    engine.on_key_press(key);
    assert!(engine.global_enabled.load(Ordering::SeqCst));
}

#[test]
fn stale_release_cannot_stop_a_new_rule_reusing_the_same_id() {
    let old = KeyId::Keyboard(81);
    let new = KeyId::Keyboard(82);
    let (engine, scheduler) = setup(vec![rule(
        "same",
        BurstMode::Hold,
        old,
        KeyId::Keyboard(69),
    )]);
    let engine = Arc::new(engine);
    engine.on_key_press(old);
    let (entered, resume) = pause_routing(&engine);
    let releasing = {
        let engine = engine.clone();
        std::thread::spawn(move || engine.on_key_release(old))
    };
    entered.recv_timeout(Duration::from_secs(2)).unwrap();
    engine.set_rules(vec![rule(
        "same",
        BurstMode::Hold,
        new,
        KeyId::Keyboard(69),
    )]);
    engine.on_key_press(new);
    scheduler.clear();
    resume.send(()).unwrap();
    releasing.join().unwrap();
    assert!(scheduler.cmds().is_empty());
    assert_eq!(engine.get_active_ids(), vec!["same"]);
}

struct BlockingStop {
    inner: crate::test_support::RecordingScheduler,
    armed: AtomicBool,
    confirmed: AtomicBool,
    entered: mpsc::Sender<()>,
    resume: Mutex<mpsc::Receiver<()>>,
}

impl Scheduler for BlockingStop {
    fn start_rule(&self, rule: Arc<BurstRule>, generation: u64) {
        self.inner.start_rule(rule, generation);
    }
    fn tap_once(&self, rule: Arc<BurstRule>, generation: u64) {
        self.inner.tap_once(rule, generation);
    }
    fn stop_rule(&self, id: String, generation: u64) {
        self.inner.stop_rule(id, generation);
    }
    fn pause_rule(&self, id: String, generation: u64) {
        self.inner.pause_rule(id, generation);
    }
    fn resume_rule(&self, id: String, generation: u64) {
        self.inner.resume_rule(id, generation);
    }
    fn play_once(&self, rule: Arc<BurstRule>, generation: u64) {
        self.inner.play_once(rule, generation);
    }
    fn stop_all_async(&self, generation: u64) {
        self.inner.stop_all_async(generation);
    }
    fn stop_all_blocking(&self, _: u64) -> bool {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.send(()).unwrap();
            self.resume
                .lock()
                .unwrap()
                .recv_timeout(Duration::from_secs(2))
                .unwrap();
        }
        self.confirmed.load(Ordering::SeqCst)
    }
    fn shutdown_blocking(&self, _: u64) -> bool {
        true
    }
    fn hp_degraded(&self) -> bool {
        false
    }
}

#[test]
fn newer_stop_and_enable_do_not_unlock_output_before_older_cleanup_finishes() {
    let (entered_tx, entered_rx) = mpsc::channel();
    let (resume_tx, resume_rx) = mpsc::channel();
    let scheduler = Arc::new(BlockingStop {
        inner: Default::default(),
        armed: AtomicBool::new(false),
        confirmed: AtomicBool::new(true),
        entered: entered_tx,
        resume: Mutex::new(resume_rx),
    });
    let engine = Arc::new(BurstEngine::new_with_scheduler(scheduler.clone()));
    let key = KeyId::Keyboard(81);
    engine.set_rules(vec![rule(
        "held",
        BurstMode::Hold,
        key,
        KeyId::Keyboard(69),
    )]);
    engine.set_global_enabled(true, false);
    scheduler.armed.store(true, Ordering::SeqCst);
    let stopping = {
        let engine = engine.clone();
        std::thread::spawn(move || engine.cancel_all_loops())
    };
    entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
    engine.cancel_all_loops();
    engine.set_global_enabled(true, false);
    scheduler.inner.clear();
    assert!(!engine.on_key_press(key));
    assert!(scheduler.inner.cmds().is_empty());
    resume_tx.send(()).unwrap();
    stopping.join().unwrap();
    engine.on_key_release(key);
    assert!(engine.on_key_press(key));
}

#[test]
fn unconfirmed_stop_rejects_backend_switch_preview_and_recording() {
    let (entered, _) = mpsc::channel();
    let (_, resume) = mpsc::channel();
    let scheduler = Arc::new(BlockingStop {
        inner: Default::default(),
        armed: AtomicBool::new(false),
        confirmed: AtomicBool::new(true),
        entered,
        resume: Mutex::new(resume),
    });
    let engine = BurstEngine::new_with_scheduler(scheduler.clone());
    engine.set_global_enabled(true, false);
    scheduler.confirmed.store(false, Ordering::SeqCst);
    assert!(engine.begin_backend_switch().is_err());
    assert!(!engine.switching_backend.load(Ordering::SeqCst));
    assert!(engine
        .preview_output(vec![qzh_profile::OutputStep {
            key: KeyId::Keyboard(69),
            interval_ms: 50
        }])
        .is_err());
    assert!(engine.start_recording(KeyId::Keyboard(123)).is_err());
    scheduler.confirmed.store(true, Ordering::SeqCst);
    assert!(engine.begin_backend_switch().is_ok());
    engine.end_backend_switch();
}
