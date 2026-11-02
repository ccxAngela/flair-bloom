use super::*;
use crate::profile::BurstMode;

fn kb(vk: u32) -> KeyId {
    KeyId::Keyboard(vk)
}

fn ms(btn: MouseButton) -> KeyId {
    KeyId::Mouse(btn)
}

const RIGHT_ALT: u32 = 0xa5;
const KEY_Q: u32 = 0x51;

/// 不支持自注入过滤的后端（DDSimple），Toggle 重合态为空集。
fn no_coincident() -> InjectCaps {
    InjectCaps {
        coincident_toggle: false,
    }
}

/// 同一条规则的长按版，用于验证重合槽位按模式分流。
fn hold_rule(trigger: KeyId, target: KeyId, stop: Option<KeyId>) -> BurstRule {
    BurstRule {
        mode: BurstMode::Hold,
        ..rule(trigger, target, stop)
    }
}

fn rule(trigger: KeyId, target: KeyId, stop: Option<KeyId>) -> BurstRule {
    BurstRule {
        id: "r1".to_string(),
        enabled: true,
        trigger_key: trigger,
        steps: vec![crate::profile::OutputStep {
            key: target,
            interval_ms: 10,
        }],
        mode: BurstMode::Toggle,
        stop_key: stop,
        group: None,
    }
}

// ── 修饰键：只有热键槽接受 ───────────────────────────────────────────────────

#[test]
fn modifier_accepted_only_in_hotkey_slot() {
    let caps = InjectCaps::default();
    assert!(accepts(KeySlot::Hotkey, kb(RIGHT_ALT), caps).is_ok());
    for slot in [KeySlot::Trigger, KeySlot::Target, KeySlot::TriggerTarget] {
        assert_eq!(
            accepts(slot, kb(RIGHT_ALT), caps),
            Err(KeyRejection::ModifierNotAllowed),
            "{slot:?} 不应接受修饰键"
        );
    }
}

#[test]
fn every_listed_modifier_is_recognized() {
    for vk in MODIFIER_VKS {
        assert!(is_modifier_vk(vk), "0x{vk:x} 应被认作修饰键");
    }
    assert!(!is_modifier_vk(KEY_Q));
}

#[test]
fn ordinary_keyboard_key_accepted_everywhere() {
    let caps = InjectCaps::default();
    for slot in [
        KeySlot::Hotkey,
        KeySlot::Trigger,
        KeySlot::Target,
        KeySlot::TriggerTarget,
    ] {
        assert!(accepts(slot, kb(KEY_Q), caps).is_ok(), "{slot:?}");
    }
}

// ── 鼠标：热键槽全禁，其余按后端能力 ─────────────────────────────────────────

#[test]
fn hotkey_slot_rejects_every_mouse_button() {
    let caps = InjectCaps::default();
    for btn in ALL_MOUSE_BUTTONS {
        assert_eq!(
            accepts(KeySlot::Hotkey, ms(btn), caps),
            Err(KeyRejection::MouseNotAllowed),
            "{btn:?}"
        );
    }
}

// ── 重合态取交集，不是并集 ───────────────────────────────────────────────────

#[test]
fn coincident_slot_is_intersection_of_read_and_write() {
    let caps = InjectCaps::default();
    for btn in ALL_MOUSE_BUTTONS {
        let readable = accepts(KeySlot::Trigger, ms(btn), caps).is_ok();
        let writable = accepts(KeySlot::Target, ms(btn), caps).is_ok();
        let both = accepts(KeySlot::TriggerTarget, ms(btn), caps).is_ok();
        assert_eq!(both, readable && writable, "{btn:?} 重合态应为交集");
    }
}

// ── 槽位导出与判定同源 ───────────────────────────────────────────────────────

#[test]
fn slot_policy_matches_accepts() {
    for caps in [InjectCaps::default(), no_coincident()] {
        for slot in [
            KeySlot::Hotkey,
            KeySlot::Trigger,
            KeySlot::Target,
            KeySlot::TriggerTarget,
            KeySlot::TriggerTargetToggle,
        ] {
            let policy = slot_policy(slot, caps);
            assert_eq!(
                policy.keyboard,
                accepts(slot, kb(0x41), caps).is_ok(),
                "{slot:?} 普通键盘键不一致"
            );
            assert_eq!(
                policy.modifiers,
                accepts(slot, kb(RIGHT_ALT), caps).is_ok(),
                "{slot:?} 修饰键位不一致"
            );
            for btn in ALL_MOUSE_BUTTONS {
                assert_eq!(
                    policy.mouse.contains(&btn),
                    accepts(slot, ms(btn), caps).is_ok(),
                    "{slot:?} {btn:?} 不一致"
                );
            }
        }
    }
}

#[test]
fn hotkey_policy_allows_modifiers_and_no_mouse() {
    let p = slot_policy(KeySlot::Hotkey, InjectCaps::default());
    assert!(p.keyboard);
    assert!(p.modifiers);
    assert!(p.mouse.is_empty());
}

/// 后端不支持重合态时，Toggle 的重合槽位是真空集：键盘键也收不了。
#[test]
fn toggle_coincident_policy_is_empty_without_caps() {
    let p = slot_policy(KeySlot::TriggerTargetToggle, no_coincident());
    assert!(!p.keyboard);
    assert!(!p.modifiers);
    assert!(p.mouse.is_empty());

    let p = slot_policy(KeySlot::TriggerTargetToggle, InjectCaps::default());
    assert!(p.keyboard);
    assert!(p.mouse.len() == ALL_MOUSE_BUTTONS.len());
}

// ── 规则里的槽位归属 ─────────────────────────────────────────────────────────

#[test]
fn distinct_trigger_and_target_split_into_pure_roles() {
    let r = rule(kb(KEY_Q), ms(MouseButton::Left), None);
    assert_eq!(slot_of(&r, kb(KEY_Q)), KeySlot::Trigger);
    assert_eq!(slot_of(&r, ms(MouseButton::Left)), KeySlot::Target);
}

/// 重合槽位按模式分流：Toggle 的自注入过滤要求更高，与 Hold 不是同一个槽位。
#[test]
fn same_trigger_and_target_is_coincident() {
    let r = rule(kb(KEY_Q), kb(KEY_Q), None);
    assert_eq!(slot_of(&r, kb(KEY_Q)), KeySlot::TriggerTargetToggle);

    let r = hold_rule(kb(KEY_Q), kb(KEY_Q), None);
    assert_eq!(slot_of(&r, kb(KEY_Q)), KeySlot::TriggerTarget);
}

#[test]
fn stop_key_equal_to_target_is_coincident() {
    let r = rule(
        kb(KEY_Q),
        ms(MouseButton::Left),
        Some(ms(MouseButton::Left)),
    );
    assert_eq!(
        slot_of(&r, ms(MouseButton::Left)),
        KeySlot::TriggerTargetToggle
    );

    let r = hold_rule(
        kb(KEY_Q),
        ms(MouseButton::Left),
        Some(ms(MouseButton::Left)),
    );
    assert_eq!(slot_of(&r, ms(MouseButton::Left)), KeySlot::TriggerTarget);
}

#[test]
fn explicit_stop_key_is_read_only() {
    let r = rule(kb(KEY_Q), ms(MouseButton::Left), Some(kb(0x52)));
    assert_eq!(slot_of(&r, kb(0x52)), KeySlot::Trigger);
}

// ── 违规扫描 ─────────────────────────────────────────────────────────────────

#[test]
fn rule_violation_found_for_modifier_target() {
    let rules = vec![rule(kb(KEY_Q), kb(RIGHT_ALT), None)];
    let found = find_rule_violations(&rules, InjectCaps::default());
    assert_eq!(
        found,
        vec![("r1".to_string(), KeyRejection::ModifierNotAllowed)]
    );
}

#[test]
fn disabled_rules_are_skipped() {
    let mut r = rule(kb(KEY_Q), kb(RIGHT_ALT), None);
    r.enabled = false;
    assert!(find_rule_violations(&[r], InjectCaps::default()).is_empty());
}

#[test]
fn duplicate_keys_reported_once_per_rule() {
    // trigger == target == stop，同一个键只该报一次。
    let rules = vec![rule(kb(RIGHT_ALT), kb(RIGHT_ALT), Some(kb(RIGHT_ALT)))];
    assert_eq!(find_rule_violations(&rules, InjectCaps::default()).len(), 1);
}

#[test]
fn clean_rules_report_nothing() {
    let rules = vec![rule(kb(KEY_Q), ms(MouseButton::Left), None)];
    assert!(find_rule_violations(&rules, InjectCaps::default()).is_empty());
}

// ── 热键净化 ─────────────────────────────────────────────────────────────────

#[test]
fn sanitize_drops_mouse_hotkeys_and_keeps_keyboard_ones() {
    let mut hk = Hotkeys {
        global_toggle: Some(ms(MouseButton::Left)),
        global_stop: Some(kb(RIGHT_ALT)),
        panel_toggle: Some(ms(MouseButton::WheelUp)),
    };
    let dropped = sanitize_hotkeys(&mut hk);
    assert_eq!(dropped, vec!["global_toggle", "panel_toggle"]);
    assert_eq!(hk.global_toggle, None);
    assert_eq!(hk.global_stop, Some(kb(RIGHT_ALT)));
    assert_eq!(hk.panel_toggle, None);
}

#[test]
fn sanitize_is_noop_for_valid_hotkeys() {
    let mut hk = Hotkeys {
        global_toggle: Some(kb(KEY_Q)),
        global_stop: None,
        panel_toggle: Some(kb(RIGHT_ALT)),
    };
    let before = hk.clone();
    assert!(sanitize_hotkeys(&mut hk).is_empty());
    assert_eq!(hk.global_toggle, before.global_toggle);
    assert_eq!(hk.panel_toggle, before.panel_toggle);
}

#[test]
fn slot_injects_flags_write_roles_only() {
    assert!(!KeySlot::Hotkey.injects());
    assert!(!KeySlot::Trigger.injects());
    assert!(KeySlot::Target.injects());
    assert!(KeySlot::TriggerTarget.injects());
}

// ── 加载期净化 ───────────────────────────────────────────────────────────────

fn profile_with(rules: Vec<BurstRule>, hotkeys: Hotkeys) -> Profile {
    Profile {
        schema_version: crate::profile::CURRENT_SCHEMA_VERSION,
        meta: crate::profile::ProfileMeta {
            name: "t".to_string(),
            created_at: 0,
            updated_at: 0,
            app_version: "0".to_string(),
            signature: String::new(),
        },
        rules,
        hotkeys,
        advanced: Default::default(),
    }
}

#[test]
fn sanitize_disables_offending_rule_without_touching_its_keys() {
    let mut p = profile_with(
        vec![rule(kb(KEY_Q), kb(RIGHT_ALT), None)],
        Hotkeys::default(),
    );
    let report = sanitize_profile(&mut p, InjectCaps::default());

    assert_eq!(report.disabled_rules.len(), 1);
    assert_eq!(report.disabled_rules[0].id, "r1");
    assert_eq!(
        report.disabled_rules[0].reason,
        KeyRejection::ModifierNotAllowed
    );
    assert!(!p.rules[0].enabled, "违规规则应被停用");
    // 按键原样保留：改写成空会造出新的中间态，连锁面太大。
    assert_eq!(p.rules[0].steps[0].key, kb(RIGHT_ALT));
    assert_eq!(p.rules[0].trigger_key, kb(KEY_Q));
}

#[test]
fn sanitize_clears_mouse_hotkey() {
    let mut p = profile_with(
        vec![],
        Hotkeys {
            global_toggle: Some(ms(MouseButton::Left)),
            global_stop: None,
            panel_toggle: None,
        },
    );
    let report = sanitize_profile(&mut p, InjectCaps::default());
    assert_eq!(report.cleared_hotkeys, vec!["global_toggle"]);
    assert_eq!(p.hotkeys.global_toggle, None);
}

#[test]
fn sanitize_reports_nothing_for_clean_profile() {
    let mut p = profile_with(
        vec![rule(kb(KEY_Q), ms(MouseButton::Left), None)],
        Hotkeys {
            global_toggle: Some(kb(RIGHT_ALT)),
            global_stop: None,
            panel_toggle: None,
        },
    );
    let report = sanitize_profile(&mut p, InjectCaps::default());
    assert!(report.is_empty());
    assert!(p.rules[0].enabled);
    assert_eq!(p.hotkeys.global_toggle, Some(kb(RIGHT_ALT)));
}

// ── DD 下 Toggle 的重合态是空集 ──────────────────────────────────────────────

#[test]
fn dd_rejects_coincident_toggle_regardless_of_key() {
    for key in [kb(KEY_Q), ms(MouseButton::Left)] {
        let rules = vec![rule(key, key, None)];
        let found = find_rule_violations(&rules, no_coincident());
        assert_eq!(
            found,
            vec![("r1".to_string(), KeyRejection::CoincidentToggleUnsupported)],
            "{key:?}"
        );
    }
}

#[test]
fn dd_rejects_toggle_stop_key_equal_to_target() {
    let rules = vec![rule(kb(KEY_Q), kb(0x52), Some(kb(0x52)))];
    let found = find_rule_violations(&rules, no_coincident());
    assert_eq!(
        found,
        vec![("r1".to_string(), KeyRejection::CoincidentToggleUnsupported)]
    );
}

#[test]
fn dd_allows_coincident_hold() {
    // Hold 的重合态不可靠但已被接受，不该拦。
    let mut r = rule(kb(KEY_Q), kb(KEY_Q), None);
    r.mode = BurstMode::Hold;
    assert!(find_rule_violations(&[r], no_coincident()).is_empty());
}

#[test]
fn dd_allows_toggle_with_distinct_keys() {
    let rules = vec![rule(kb(KEY_Q), kb(0x52), None)];
    assert!(find_rule_violations(&rules, no_coincident()).is_empty());
}

#[test]
fn every_mouse_button_is_injectable_on_current_backends() {
    // DD-HID 退役后已无值域缺口的后端，写角色接受全部鼠标按钮与滚轮。
    for caps in [InjectCaps::default(), no_coincident()] {
        for btn in ALL_MOUSE_BUTTONS {
            assert!(accepts(KeySlot::Target, ms(btn), caps).is_ok(), "{btn:?}");
        }
    }
}
