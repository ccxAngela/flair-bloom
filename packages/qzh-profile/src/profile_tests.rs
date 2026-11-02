use super::*;
use crate::key_id::{KeyId, MouseButton};
use serde_json::json;

fn make_profile(rules: Vec<BurstRule>) -> Profile {
    Profile {
        schema_version: CURRENT_SCHEMA_VERSION,
        meta: ProfileMeta {
            name: "t".into(),
            created_at: 0,
            updated_at: 0,
            app_version: "0".into(),
            signature: String::new(),
        },
        rules,
        hotkeys: Hotkeys::default(),
        advanced: Advanced::default(),
    }
}

fn rule(id: &str, mode: BurstMode, trigger: KeyId, target: KeyId, interval: u32) -> BurstRule {
    BurstRule {
        id: id.into(),
        enabled: true,
        trigger_key: trigger,
        steps: vec![crate::profile::OutputStep {
            key: target,
            interval_ms: interval,
        }],
        mode,
        stop_key: None,
        group: None,
    }
}

fn kbd(vk: u32) -> KeyId {
    KeyId::Keyboard(vk)
}

#[test]
fn validate_accepts_empty_profile() {
    assert!(make_profile(vec![]).validate().is_ok());
}

#[test]
fn validate_accepts_interval_at_lower_bound() {
    let p = make_profile(vec![rule(
        "r",
        BurstMode::Hold,
        kbd(0x41),
        kbd(0x42),
        MIN_INTERVAL_MS,
    )]);
    assert!(p.validate().is_ok());
}

#[test]
fn validate_accepts_interval_at_upper_bound() {
    let p = make_profile(vec![rule(
        "r",
        BurstMode::Hold,
        kbd(0x41),
        kbd(0x42),
        MAX_INTERVAL_MS,
    )]);
    assert!(p.validate().is_ok());
}

#[test]
fn validate_rejects_interval_below_minimum() {
    let p = make_profile(vec![rule("r", BurstMode::Hold, kbd(0x41), kbd(0x42), 0)]);
    assert!(matches!(
        p.validate(),
        Err(ProfileError::InvalidInterval(0))
    ));
}

#[test]
fn validate_rejects_interval_above_maximum() {
    let p = make_profile(vec![rule(
        "r",
        BurstMode::Hold,
        kbd(0x41),
        kbd(0x42),
        MAX_INTERVAL_MS + 1,
    )]);
    assert!(matches!(
        p.validate(),
        Err(ProfileError::InvalidInterval(i)) if i == MAX_INTERVAL_MS + 1
    ));
}

#[test]
fn validate_rejects_too_many_rules() {
    let rules = (0..=MAX_RULES)
        .map(|i| rule(&format!("r{i}"), BurstMode::Hold, kbd(0x41), kbd(0x42), 10))
        .collect();
    assert!(matches!(
        make_profile(rules).validate(),
        Err(ProfileError::TooManyRules)
    ));
}

#[test]
fn validate_accepts_max_rules() {
    let rules = (0..MAX_RULES)
        .map(|i| rule(&format!("r{i}"), BurstMode::Hold, kbd(0x41), kbd(0x42), 10))
        .collect();
    assert!(make_profile(rules).validate().is_ok());
}

#[test]
fn validate_default_mode_allows_toggle_target_equals_trigger() {
    // 非 DD 模式（distinct_target = false）允许 toggle target == trigger
    let p = make_profile(vec![rule("t", BurstMode::Toggle, kbd(0x46), kbd(0x46), 10)]);
    assert!(p.validate().is_ok());
}

#[test]
fn burst_rule_serializes_keyid_shape() {
    let r = rule(
        "x",
        BurstMode::Hold,
        kbd(0x51),
        KeyId::Mouse(MouseButton::Left),
        10,
    );
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["trigger_key"], json!({"kind":"keyboard","code":81}));
    assert_eq!(v["steps"][0]["key"], json!({"kind":"mouse","code":"left"}));
    assert!(v.get("target_key").is_none());
}

#[test]
fn legacy_rule_becomes_a_single_step_and_old_reader_rejects_sequences() {
    #[derive(serde::Deserialize)]
    struct LegacyRule {
        #[serde(rename = "target_key")]
        _target_key: KeyId,
        #[serde(rename = "interval_ms")]
        _interval_ms: u32,
    }
    let legacy = json!({"id":"old", "enabled":true, "trigger_key":{"kind":"keyboard","code":81},
        "target_key":{"kind":"keyboard","code":69}, "interval_ms":50, "mode":"toggle"});
    assert!(serde_json::from_value::<LegacyRule>(legacy.clone()).is_ok());
    let mut loaded: BurstRule = serde_json::from_value(legacy).unwrap();
    assert_eq!(
        loaded.steps,
        vec![OutputStep {
            key: kbd(69),
            interval_ms: 50
        }]
    );
    loaded.steps.push(OutputStep {
        key: kbd(82),
        interval_ms: 1500,
    });
    let written = serde_json::to_value(&loaded).unwrap();
    assert!(serde_json::from_value::<LegacyRule>(written.clone()).is_err());
    assert_eq!(
        serde_json::from_value::<BurstRule>(written).unwrap().steps,
        loaded.steps
    );
}

#[test]
fn rejects_empty_oversized_mixed_and_invalid_later_steps() {
    let mut r = rule("x", BurstMode::Toggle, kbd(81), kbd(69), 50);
    r.steps.clear();
    assert!(matches!(
        r.validate(),
        Err(ProfileError::InvalidStepCount(0))
    ));
    r.steps = vec![
        OutputStep {
            key: kbd(69),
            interval_ms: 50
        };
        MAX_OUTPUT_STEPS + 1
    ];
    assert!(r.validate().is_err());
    r.steps = vec![
        OutputStep {
            key: kbd(69),
            interval_ms: 50,
        },
        OutputStep {
            key: kbd(82),
            interval_ms: 0,
        },
    ];
    assert!(matches!(
        r.validate(),
        Err(ProfileError::InvalidInterval(0))
    ));
    r.steps.pop();
    let mut mixed = serde_json::to_value(&r).unwrap();
    mixed["target_key"] = json!({"kind":"keyboard","code":70});
    mixed["interval_ms"] = json!(10);
    assert!(serde_json::from_value::<BurstRule>(mixed).is_err());
}

#[test]
fn sanitize_signature_strips_controls_trims_and_truncates_by_chars() {
    assert_eq!(sanitize_signature("  花间\u{7}游悟\n  "), "花间游悟");
    let long: String = "署".repeat(MAX_SIGNATURE_CHARS + 5);
    assert_eq!(
        sanitize_signature(&long).chars().count(),
        MAX_SIGNATURE_CHARS
    );
    // 截断后落在空格上时不留尾随空白
    let padded = format!("{} 尾", "a".repeat(MAX_SIGNATURE_CHARS - 1));
    assert_eq!(
        sanitize_signature(&padded),
        "a".repeat(MAX_SIGNATURE_CHARS - 1)
    );
    assert_eq!(sanitize_signature("\u{1b}[31m"), "[31m");
}

#[test]
fn meta_without_signature_deserializes_to_empty() {
    let meta: ProfileMeta = serde_json::from_value(json!({
        "name": "a", "created_at": 1, "updated_at": 2, "app_version": "0.5.0"
    }))
    .unwrap();
    assert_eq!(meta.signature, "");
}

#[test]
fn meta_signature_is_sanitized_on_deserialize() {
    let meta: ProfileMeta = serde_json::from_value(json!({
        "name": "a", "created_at": 1, "updated_at": 2, "app_version": "0.5.0",
        "signature": format!("\t{}\u{0}", "长".repeat(40)),
    }))
    .unwrap();
    assert_eq!(meta.signature, "长".repeat(MAX_SIGNATURE_CHARS));
}

#[test]
fn meta_signature_round_trips() {
    let mut p = make_profile(vec![]);
    p.meta.signature = "亲友 · 小花".into();
    let back: Profile = serde_json::from_value(serde_json::to_value(&p).unwrap()).unwrap();
    assert_eq!(back.meta.signature, "亲友 · 小花");
}

#[test]
fn sanitize_signature_strips_bidi_and_zero_width() {
    assert_eq!(
        sanitize_signature("\u{202E}花小\u{200B}\u{2066}x\u{FEFF}"),
        "花小x"
    );
    assert_eq!(sanitize_signature("\u{200B}\u{200B}"), "");
}

#[test]
fn meta_null_signature_deserializes_to_empty() {
    let meta: ProfileMeta = serde_json::from_value(json!({
        "name": "a", "created_at": 1, "updated_at": 2, "app_version": "0.5.0", "signature": null
    }))
    .unwrap();
    assert_eq!(meta.signature, "");
}
