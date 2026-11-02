use super::*;
use crate::scheduler::EventDispatcher;
use crate::test_support::{rule, setup};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use win_input::{DispatchResult, InputEvent};

#[test]
fn dedicated_stop_cancels_recording_even_when_output_is_disabled_or_key_is_duplicated() {
    for duplicate in [false, true] {
        let (engine, _) = setup(vec![]);
        engine.set_global_enabled(false, false);
        let stop = KeyId::Keyboard(122);
        engine.set_hotkeys(Hotkeys {
            global_toggle: Some(KeyId::Keyboard(121)),
            global_stop: Some(stop),
            panel_toggle: None,
        });
        engine.start_recording(KeyId::Keyboard(123)).unwrap();
        if duplicate {
            revive(engine.physical_pressed.lock()).insert(stop);
        }
        engine.on_key_press(stop);
        let snapshot = engine.get_recording();
        assert!(!snapshot.recording);
        assert!(snapshot.steps.is_empty());
        assert!(!engine.global_enabled.load(Ordering::SeqCst));
    }
}

#[test]
fn shared_global_toggle_cancels_recording_without_enabling_output() {
    let (engine, _) = setup(vec![]);
    engine.set_global_enabled(false, false);
    let toggle = KeyId::Keyboard(122);
    engine.set_hotkeys(Hotkeys {
        global_toggle: Some(toggle),
        ..Default::default()
    });
    engine.start_recording(KeyId::Keyboard(123)).unwrap();
    engine.on_key_press(toggle);
    let snapshot = engine.get_recording();
    assert!(!snapshot.recording);
    assert!(snapshot.steps.is_empty());
    assert!(!engine.global_enabled.load(Ordering::SeqCst));
}

struct FailPauseRelease {
    down: mpsc::Sender<()>,
    fail_up: AtomicBool,
}

impl EventDispatcher for FailPauseRelease {
    fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
        events
            .iter()
            .map(|event| {
                if event.key == KeyId::Keyboard(69) {
                    if !event.is_up {
                        let _ = self.down.send(());
                    } else if self.fail_up.swap(false, Ordering::SeqCst) {
                        return DispatchResult::Failed;
                    }
                }
                DispatchResult::Sent
            })
            .collect()
    }
}

#[test]
fn failed_paused_queue_never_restarts_when_hold_is_released_before_ui_poll() {
    let (down_tx, down_rx) = mpsc::channel();
    let dispatcher = Arc::new(FailPauseRelease {
        down: down_tx,
        fail_up: AtomicBool::new(true),
    });
    let simulated = SimulatedKeys::default();
    let scheduler = Arc::new(SchedulerHandle::start_with_dispatcher(
        dispatcher,
        None,
        simulated.clone(),
    ));
    let engine = BurstEngine::from_parts(scheduler, simulated);
    let mut toggle = rule(
        "toggle",
        BurstMode::Toggle,
        KeyId::Keyboard(81),
        KeyId::Keyboard(69),
    );
    toggle.group = Some("g".into());
    toggle.steps[0].interval_ms = 10000;
    let mut hold = rule(
        "hold",
        BurstMode::Hold,
        KeyId::Keyboard(72),
        KeyId::Keyboard(82),
    );
    hold.group = Some("g".into());
    engine.set_rules(vec![toggle, hold]);
    engine.set_global_enabled(true, false);
    engine.on_key_press(KeyId::Keyboard(81));
    engine.on_key_release(KeyId::Keyboard(81));
    down_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    engine.on_key_press(KeyId::Keyboard(72));
    let deadline = Instant::now() + Duration::from_secs(1);
    loop {
        if engine
            .get_rule_progress()
            .iter()
            .any(|p| p.rule_id == "toggle" && p.error.is_some())
        {
            break;
        }
        assert!(Instant::now() < deadline, "没有进入预期的暂停释放失败状态");
        std::thread::yield_now();
    }
    engine.on_key_release(KeyId::Keyboard(72));
    assert!(down_rx.recv_timeout(Duration::from_millis(100)).is_err());
    assert!(engine
        .get_rule_progress()
        .iter()
        .any(|p| p.rule_id == "toggle" && p.error.is_some()));
    assert!(!engine.get_rule_states().running.contains(&"toggle".into()));
}
