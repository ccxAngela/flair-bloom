use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender};
use std::time::{SystemTime, UNIX_EPOCH};

/// 只传成功输出的发生时间；代际隔离关闭前的在途观察，不携带规则或按键。
pub struct RuleActivity {
    generation: AtomicU64,
    observed: AtomicU64,
    tx: SyncSender<ActivityObservation>,
}

#[derive(Clone, Copy, Debug)]
pub struct ActivityObservation {
    pub generation: u64,
    pub occurred_at: SystemTime,
}

impl RuleActivity {
    pub fn channel() -> (Self, Receiver<ActivityObservation>) {
        let (tx, rx) = sync_channel(16);
        (
            Self {
                generation: AtomicU64::new(0),
                observed: AtomicU64::new(0),
                tx,
            },
            rx,
        )
    }

    /// 偶数代关闭，奇数代开启；每次变更都使先前的观察失效。
    pub fn set_enabled(&self, enabled: bool) -> u64 {
        let old = self.generation.load(Ordering::SeqCst);
        let generation = (old + 2) & !1 | u64::from(enabled);
        self.generation.store(generation, Ordering::SeqCst);
        generation
    }

    pub(crate) fn record(&self, now: SystemTime) {
        let generation = self.generation.load(Ordering::SeqCst);
        if generation & 1 == 0 {
            return;
        }
        let Ok(elapsed) = now.duration_since(UNIX_EPOCH) else {
            return;
        };
        let day = (elapsed.as_secs() + 8 * 3600) / 86_400;
        let stamp = (generation << 32) | day;
        let previous = self.observed.swap(stamp, Ordering::Relaxed);
        if previous == stamp {
            return;
        }
        // 满载时下次真实输出再试，不阻塞调度器，也不无限缓存。
        if self
            .tx
            .try_send(ActivityObservation {
                generation,
                occurred_at: now,
            })
            .is_err()
        {
            let _ = self.observed.compare_exchange(
                stamp,
                previous,
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn only_observes_enabled_generations_and_shanghai_days() {
        let (activity, rx) = RuleActivity::channel();
        let midnight = UNIX_EPOCH + Duration::from_secs(20_000 * 86_400 - 8 * 3600);
        activity.record(midnight);
        assert!(rx.try_recv().is_err());
        let first = activity.set_enabled(true);
        activity.record(midnight - Duration::from_millis(1));
        activity.record(midnight - Duration::from_millis(1));
        activity.record(midnight);
        assert_eq!(
            rx.try_iter()
                .map(|item| item.generation)
                .collect::<Vec<_>>(),
            vec![first, first]
        );
        activity.set_enabled(false);
        activity.record(midnight);
        assert!(rx.try_recv().is_err());
        let next = activity.set_enabled(true);
        activity.record(midnight);
        assert_eq!(rx.try_recv().unwrap().generation, next);
    }

    #[test]
    fn concurrent_outputs_emit_one_observation_per_day() {
        let (activity, rx) = RuleActivity::channel();
        activity.set_enabled(true);
        std::thread::scope(|scope| {
            for _ in 0..8 {
                scope.spawn(|| {
                    for _ in 0..100 {
                        activity.record(UNIX_EPOCH + Duration::from_secs(1_000_000));
                    }
                });
            }
        });
        assert_eq!(rx.try_iter().count(), 1);
    }
}
