use qzh_profile::key_id::KeyId;
use qzh_profile::profile::{BurstMode, BurstRule};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, Default)]
pub struct RuleSnapshot {
    rules: Vec<Arc<BurstRule>>,
    press_index: HashMap<KeyId, Vec<Arc<BurstRule>>>,
    hold_release_index: HashMap<KeyId, Vec<Arc<BurstRule>>>,
}

impl RuleSnapshot {
    pub fn new(rules: Vec<BurstRule>) -> Self {
        let rules: Vec<_> = rules
            .into_iter()
            .filter(|r| r.validate().is_ok())
            .map(Arc::new)
            .collect();
        let mut press_index: HashMap<KeyId, Vec<Arc<BurstRule>>> = HashMap::new();
        let mut hold_release_index: HashMap<KeyId, Vec<Arc<BurstRule>>> = HashMap::new();

        for rule in rules.iter().filter(|r| r.enabled) {
            match rule.mode {
                BurstMode::Hold => {
                    press_index
                        .entry(rule.trigger_key)
                        .or_default()
                        .push(rule.clone());
                    // 滚轮触发键没有「松开」事件（每格瞬发 press+release），其 Hold 规则由引擎
                    // 当作一次性点按处理、不靠 release 停止；若放进 hold_release 索引，紧随 press
                    // 的合成 release 会在调度器发出首拍前就把规则停掉（零注入）。故排除滚轮。
                    if !rule.trigger_key.is_wheel() {
                        hold_release_index
                            .entry(rule.trigger_key)
                            .or_default()
                            .push(rule.clone());
                    }
                }
                BurstMode::Toggle => {
                    press_index
                        .entry(rule.trigger_key)
                        .or_default()
                        .push(rule.clone());
                    let stop = rule.stop_key.unwrap_or(rule.trigger_key);
                    if stop != rule.trigger_key {
                        press_index.entry(stop).or_default().push(rule.clone());
                    }
                }
            }
        }

        Self {
            rules,
            press_index,
            hold_release_index,
        }
    }

    pub fn rules(&self) -> Vec<BurstRule> {
        self.rules.iter().map(|r| (**r).clone()).collect()
    }

    pub fn enabled_press_rules(&self, key: KeyId) -> Vec<Arc<BurstRule>> {
        self.press_index.get(&key).cloned().unwrap_or_default()
    }

    pub fn enabled_hold_release_rules(&self, key: KeyId) -> Vec<Arc<BurstRule>> {
        self.hold_release_index
            .get(&key)
            .cloned()
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(id: &str, mode: BurstMode, trigger_key: KeyId, target_key: KeyId) -> BurstRule {
        BurstRule {
            id: id.to_string(),
            enabled: true,
            trigger_key,
            steps: vec![qzh_profile::profile::OutputStep {
                key: target_key,
                interval_ms: 10,
            }],
            mode,
            stop_key: None,
            group: None,
        }
    }

    #[test]
    fn toggle_with_same_start_stop_is_indexed_once() {
        let key = KeyId::Keyboard(0x51);
        let snapshot = RuleSnapshot::new(vec![rule("r", BurstMode::Toggle, key, key)]);

        assert_eq!(snapshot.enabled_press_rules(key).len(), 1);
    }

    #[test]
    fn wheel_triggered_hold_is_in_press_index_but_not_hold_release_index() {
        // 边界（A1）：滚轮触发的 Hold 规则要能被按下命中（press 索引），但不能进 hold_release
        // 索引——否则滚轮每格紧随 press 的合成 release 会把规则停在首拍之前，导致零注入。
        use qzh_profile::key_id::MouseButton;
        let wheel = KeyId::Mouse(MouseButton::WheelUp);
        let snapshot = RuleSnapshot::new(vec![rule(
            "w",
            BurstMode::Hold,
            wheel,
            KeyId::Keyboard(0x45),
        )]);

        assert_eq!(snapshot.enabled_press_rules(wheel).len(), 1);
        assert!(snapshot.enabled_hold_release_rules(wheel).is_empty());
    }

    #[test]
    fn hold_release_index_only_contains_hold_rules() {
        let hold_key = KeyId::Keyboard(0x51);
        let toggle_key = KeyId::Keyboard(0x45);
        let snapshot = RuleSnapshot::new(vec![
            rule("h", BurstMode::Hold, hold_key, KeyId::Keyboard(0x41)),
            rule("t", BurstMode::Toggle, toggle_key, KeyId::Keyboard(0x42)),
        ]);

        assert_eq!(snapshot.enabled_hold_release_rules(hold_key).len(), 1);
        assert!(snapshot.enabled_hold_release_rules(toggle_key).is_empty());
    }
}
