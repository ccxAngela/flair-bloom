use qzh_profile::{
    key_policy::is_modifier_vk, KeyId, OutputStep, MAX_INTERVAL_MS, MAX_OUTPUT_STEPS,
    MIN_EFFECTIVE_INTERVAL_MS,
};
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct RecordingSnapshot {
    pub session_id: u64,
    pub recording: bool,
    pub steps: Vec<OutputStep>,
    pub notice: Option<String>,
}

#[derive(Default)]
pub(crate) struct Recorder {
    snapshot: RecordingSnapshot,
    stop_key: Option<KeyId>,
    down: Option<KeyId>,
    last_down_at: Option<Instant>,
    started_at: Option<Instant>,
}

impl Recorder {
    pub fn start(&mut self, stop_key: KeyId, now: Instant) -> Result<RecordingSnapshot, String> {
        if self.snapshot.recording {
            return Err("已有录制正在进行".into());
        }
        self.snapshot = RecordingSnapshot {
            session_id: self.snapshot.session_id.saturating_add(1),
            recording: true,
            ..Default::default()
        };
        self.stop_key = Some(stop_key);
        self.down = None;
        self.last_down_at = None;
        self.started_at = Some(now);
        Ok(self.snapshot.clone())
    }

    fn finish(&mut self, notice: Option<&str>) {
        self.snapshot.recording = false;
        self.snapshot.notice = notice.map(str::to_owned);
        self.down = None;
    }

    pub fn stop(&mut self, session_id: u64) -> Result<RecordingSnapshot, String> {
        if session_id != self.snapshot.session_id {
            return Err("录制会话已失效".into());
        }
        if self.snapshot.recording {
            let notice = self
                .down
                .map(|_| "结束时仍有按键未松开；输出会按点按播放，请检查最后一步");
            self.finish(notice);
        }
        Ok(self.snapshot.clone())
    }

    pub fn cancel(&mut self) {
        if self.snapshot.recording {
            self.finish(Some("录制已取消，尚未保存的规则未改变"));
        }
    }

    pub fn snapshot(&mut self, now: Instant) -> RecordingSnapshot {
        self.expire(now);
        self.snapshot.clone()
    }

    fn expire(&mut self, now: Instant) {
        if self.snapshot.recording
            && self.started_at.is_some_and(|start| {
                now.saturating_duration_since(start) >= Duration::from_secs(300)
            })
        {
            self.finish(Some("已达到 5 分钟录制上限，请检查步骤"));
        }
    }

    pub fn is_recording(&self) -> bool {
        self.snapshot.recording
    }

    pub fn input(&mut self, key: KeyId, is_up: bool, now: Instant) {
        self.expire(now);
        if !self.snapshot.recording {
            return;
        }
        if !is_up && Some(key) == self.stop_key {
            let _ = self.stop(self.snapshot.session_id);
            return;
        }
        if is_up {
            if self.down == Some(key) {
                self.down = None;
            }
            return;
        }
        if !matches!(key, KeyId::Keyboard(vk) if !is_modifier_vk(vk)) {
            self.finish(Some(
                "本次录制仅支持普通键盘点按；遇到鼠标或修饰键已结束，请检查已录步骤",
            ));
            return;
        }
        if self.down.is_some() {
            self.finish(Some("检测到重叠按键，已结束录制；请检查已录步骤"));
            return;
        }
        if let Some(previous) = self.last_down_at {
            let interval = now.saturating_duration_since(previous).as_millis();
            if !(u128::from(MIN_EFFECTIVE_INTERVAL_MS)..=u128::from(MAX_INTERVAL_MS))
                .contains(&interval)
            {
                self.finish(Some(
                    "相邻按键间隔超出 10～10000ms，已结束录制；请手工修正队列",
                ));
                return;
            }
            if let Some(last) = self.snapshot.steps.last_mut() {
                last.interval_ms = interval as u32;
            }
        }
        self.snapshot.steps.push(OutputStep {
            key,
            interval_ms: MIN_EFFECTIVE_INTERVAL_MS,
        });
        self.last_down_at = Some(now);
        self.down = Some(key);
        if self.snapshot.steps.len() == MAX_OUTPUT_STEPS {
            self.finish(Some("已达到 256 步上限，请检查最后一步"));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const Q: KeyId = KeyId::Keyboard(81);
    const E: KeyId = KeyId::Keyboard(69);
    const STOP: KeyId = KeyId::Keyboard(123);

    #[test]
    fn repeated_taps_keep_order_and_stop_key_is_excluded() {
        let mut recorder = Recorder::default();
        let now = Instant::now();
        recorder.start(STOP, now).unwrap();
        for (ms, key, is_up) in [
            (0, Q, false),
            (20, Q, true),
            (50, Q, false),
            (70, Q, true),
            (130, E, false),
            (150, E, true),
            (999, STOP, false),
        ] {
            recorder.input(key, is_up, now + Duration::from_millis(ms));
        }
        let snapshot = recorder.snapshot(now + Duration::from_secs(1));
        assert!(!snapshot.recording);
        assert_eq!(
            snapshot.steps,
            vec![
                OutputStep {
                    key: Q,
                    interval_ms: 50
                },
                OutputStep {
                    key: Q,
                    interval_ms: 80
                },
                OutputStep {
                    key: E,
                    interval_ms: 10
                }
            ]
        );
    }

    #[test]
    fn overlapping_keys_end_recording_and_old_session_cannot_stop_new_one() {
        let mut recorder = Recorder::default();
        let now = Instant::now();
        let first = recorder.start(STOP, now).unwrap();
        recorder.input(Q, false, now);
        recorder.input(E, false, now + Duration::from_millis(20));
        assert!(recorder.snapshot(now).notice.is_some());
        let second = recorder.start(STOP, now).unwrap();
        assert!(recorder.stop(first.session_id).is_err());
        assert!(recorder.snapshot(now).recording);
        assert_ne!(second.session_id, first.session_id);
    }

    #[test]
    fn idle_recording_expires_without_generating_steps() {
        let mut recorder = Recorder::default();
        let now = Instant::now();
        recorder.start(STOP, now).unwrap();
        assert!(!recorder.snapshot(now + Duration::from_secs(301)).recording);
        assert!(recorder.snapshot(now).steps.is_empty());
    }
}
