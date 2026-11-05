use std::collections::BTreeMap;
const DAY: i64 = 86_400_000;
#[derive(Default)]
pub struct UsageClock {
    days: BTreeMap<i64, u64>,
}
impl UsageClock {
    pub fn observe(&mut self, now: i64, delta: u64) -> Vec<(i64, u64)> {
        let mut from = now - delta.min(7 * DAY as u64) as i64;
        let mut result = Vec::new();
        while from < now {
            let day = (from + 8 * 3_600_000).div_euclid(DAY);
            let to = now.min((day + 1) * DAY - 8 * 3_600_000);
            let total = self.days.entry(day).or_default();
            *total = (*total + (to - from) as u64).min(DAY as u64);
            result.push((to - 1, *total));
            from = to;
        }
        let oldest = (now + 8 * 3_600_000).div_euclid(DAY) - 7;
        self.days.retain(|day, _| *day >= oldest);
        result
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn splits_midnight_and_checkpoints_are_cumulative() {
        let midnight = DAY - 8 * 3_600_000;
        let mut clock = UsageClock::default();
        assert_eq!(
            clock.observe(midnight + 500, 1000),
            vec![(midnight - 1, 500), (midnight + 499, 500)]
        );
        assert_eq!(
            clock.observe(midnight + 1500, 1000),
            vec![(midnight + 1499, 1500)]
        );
        assert!(clock.observe(midnight + 1500, 0).is_empty());
    }
}
