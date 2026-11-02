use qzh_profile::{KeyId, OutputStep, MAX_INTERVAL_MS, MAX_OUTPUT_STEPS, MIN_INTERVAL_MS};
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum QueueAction {
    Press(KeyId),
    Release(KeyId),
    Tap(KeyId),
}

impl QueueAction {
    pub fn key(self) -> KeyId {
        match self {
            Self::Press(key) | Self::Release(key) | Self::Tap(key) => key,
        }
    }
    pub fn is_release(self) -> bool {
        matches!(self, Self::Release(_))
    }
}

#[derive(Debug)]
enum Phase {
    Ready,
    Pressed {
        release_at: Instant,
        next_step_at: Instant,
    },
}

#[derive(Debug)]
pub(crate) struct OutputQueue {
    steps: Arc<[OutputStep]>,
    index: usize,
    cycles: u64,
    phase: Phase,
    not_before: Instant,
    nominal_at: Instant,
    paused: Option<(Duration, Duration)>,
    last_lateness: Duration,
    max_lateness: Duration,
}

impl OutputQueue {
    pub fn new(steps: Vec<OutputStep>, now: Instant) -> Result<Self, &'static str> {
        if steps.is_empty()
            || steps.len() > MAX_OUTPUT_STEPS
            || steps
                .iter()
                .any(|step| !(MIN_INTERVAL_MS..=MAX_INTERVAL_MS).contains(&step.interval_ms))
        {
            return Err("队列步骤或间隔无效");
        }
        Ok(Self {
            steps: steps.into(),
            index: 0,
            cycles: 0,
            phase: Phase::Ready,
            not_before: now,
            nominal_at: now,
            paused: None,
            last_lateness: Duration::ZERO,
            max_lateness: Duration::ZERO,
        })
    }

    pub fn step_index(&self) -> usize {
        self.index
    }
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }
    pub fn completed_cycles(&self) -> u64 {
        self.cycles
    }
    pub fn paused_remaining(&self) -> Option<Duration> {
        self.paused.map(|(remaining, _)| remaining)
    }
    pub fn lateness(&self) -> (Duration, Duration) {
        (self.last_lateness, self.max_lateness)
    }
    pub fn held_key(&self) -> Option<KeyId> {
        matches!(self.phase, Phase::Pressed { .. }).then_some(self.steps[self.index].key)
    }

    pub fn deadline(&self) -> Option<Instant> {
        if self.paused.is_some() {
            return None;
        }
        Some(match self.phase {
            Phase::Ready => self.not_before,
            Phase::Pressed { release_at, .. } => release_at,
        })
    }

    pub fn peek(&self, now: Instant) -> Option<QueueAction> {
        if now < self.deadline()? {
            return None;
        }
        let step = self.steps[self.index];
        Some(match self.phase {
            Phase::Pressed { .. } => QueueAction::Release(step.key),
            Phase::Ready if step.key.is_wheel() || hold_duration(step.interval_ms).is_zero() => {
                QueueAction::Tap(step.key)
            }
            Phase::Ready => QueueAction::Press(step.key),
        })
    }

    pub fn acknowledge(
        &mut self,
        action: QueueAction,
        completed_at: Instant,
    ) -> Result<(), &'static str> {
        if self.peek(completed_at) != Some(action) {
            return Err("队列回报与当前动作或截止时间不符");
        }
        let step = self.steps[self.index];
        match action {
            QueueAction::Press(_) | QueueAction::Tap(_) => {
                self.last_lateness = completed_at.saturating_duration_since(self.nominal_at);
                self.max_lateness = self.max_lateness.max(self.last_lateness);
                // 从成功完成发送后重新计时，迟到不压缩后续间隔，也不按旧时刻追赶。
                let next_step_at =
                    completed_at + Duration::from_millis(u64::from(step.interval_ms));
                if matches!(action, QueueAction::Press(_)) {
                    self.phase = Phase::Pressed {
                        release_at: completed_at + hold_duration(step.interval_ms),
                        next_step_at,
                    };
                } else {
                    self.advance(next_step_at, next_step_at);
                }
            }
            QueueAction::Release(_) => {
                if let Phase::Pressed { next_step_at, .. } = self.phase {
                    self.advance(
                        next_step_at.max(completed_at + Duration::from_millis(1)),
                        next_step_at,
                    );
                }
            }
        }
        Ok(())
    }

    fn advance(&mut self, not_before: Instant, nominal_at: Instant) {
        self.index += 1;
        if self.index == self.steps.len() {
            self.index = 0;
            self.cycles = self.cycles.saturating_add(1);
        }
        self.phase = Phase::Ready;
        self.not_before = not_before;
        self.nominal_at = nominal_at;
    }

    /// 调用方先释放当前按键；暂停只冻结等待，不把已成功发送的点按再执行一次。
    pub fn pause(&mut self, now: Instant) {
        if self.paused.is_some() {
            return;
        }
        if let Phase::Pressed { next_step_at, .. } = self.phase {
            self.advance(next_step_at, next_step_at);
        }
        self.paused = Some((
            self.not_before.saturating_duration_since(now),
            self.nominal_at.saturating_duration_since(now),
        ));
    }

    pub fn resume(&mut self, now: Instant) {
        if let Some((remaining, nominal_remaining)) = self.paused.take() {
            self.not_before = now + remaining;
            self.nominal_at = now + nominal_remaining;
        }
    }
}

pub(crate) fn hold_duration(interval_ms: u32) -> Duration {
    if interval_ms <= 1 {
        return Duration::ZERO;
    }
    let interval = u64::from(interval_ms);
    Duration::from_millis((interval / 3).clamp(1, 30).min(interval - 1))
}

#[cfg(test)]
mod tests {
    use super::*;
    const Q: KeyId = KeyId::Keyboard(81);
    const E: KeyId = KeyId::Keyboard(69);

    #[test]
    fn deadline_is_a_hard_boundary_and_late_completion_never_shortens_the_next_interval() {
        let start = Instant::now();
        let mut queue = OutputQueue::new(
            vec![
                OutputStep {
                    key: Q,
                    interval_ms: 50,
                },
                OutputStep {
                    key: E,
                    interval_ms: 80,
                },
            ],
            start,
        )
        .unwrap();
        queue
            .acknowledge(QueueAction::Press(Q), start + Duration::from_millis(3))
            .unwrap();
        let release_at = queue.deadline().unwrap();
        assert!(queue.peek(release_at - Duration::from_nanos(1)).is_none());
        queue
            .acknowledge(QueueAction::Release(Q), release_at)
            .unwrap();
        let second_at = start + Duration::from_millis(53);
        assert!(queue.peek(second_at - Duration::from_nanos(1)).is_none());
        assert_eq!(queue.peek(second_at), Some(QueueAction::Press(E)));
        queue
            .acknowledge(QueueAction::Press(E), second_at + Duration::from_millis(7))
            .unwrap();
        queue
            .acknowledge(QueueAction::Release(E), queue.deadline().unwrap())
            .unwrap();
        assert!(queue
            .peek(start + Duration::from_millis(140) - Duration::from_nanos(1))
            .is_none());
        assert_eq!(
            queue.peek(start + Duration::from_millis(140)),
            Some(QueueAction::Press(Q))
        );
        assert_eq!(queue.completed_cycles(), 1);
        assert_eq!(
            queue.lateness(),
            (Duration::from_millis(7), Duration::from_millis(7))
        );
    }

    #[test]
    fn arbitrary_lateness_keeps_every_step_and_does_not_catch_up() {
        let start = Instant::now();
        let mut queue = OutputQueue::new(
            vec![
                OutputStep {
                    key: Q,
                    interval_ms: 10,
                },
                OutputStep {
                    key: E,
                    interval_ms: 10,
                },
            ],
            start,
        )
        .unwrap();
        let late = start + Duration::from_secs(60);
        assert_eq!(queue.peek(late), Some(QueueAction::Press(Q)));
        queue.acknowledge(QueueAction::Press(Q), late).unwrap();
        queue
            .acknowledge(QueueAction::Release(Q), late + Duration::from_millis(3))
            .unwrap();
        assert!(queue.peek(late + Duration::from_millis(9)).is_none());
        assert_eq!(
            queue.peek(late + Duration::from_millis(10)),
            Some(QueueAction::Press(E))
        );
        assert_eq!(queue.step_index(), 1);
    }
}
