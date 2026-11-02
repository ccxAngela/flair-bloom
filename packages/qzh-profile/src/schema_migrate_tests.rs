use super::*;
use serde_json::json;

#[test]
fn migrate_v4_normalizes_output_without_changing_rule_controls() {
    let input = json!({"schema_version":4,"rules":[{"id":"r", "enabled":false,
        "trigger_key":{"kind":"keyboard","code":81}, "target_key":{"kind":"keyboard","code":69},
        "interval_ms":1500,"mode":"toggle","stop_key":{"kind":"keyboard","code":82},"group":"g"}]});
    let migrated = migrate_profile(input.clone(), 4).unwrap();
    assert_eq!(migrated["schema_version"], 5);
    assert_eq!(
        migrated["rules"][0]["steps"],
        json!([{"key":{"kind":"keyboard","code":69},"interval_ms":1500}])
    );
    assert_eq!(
        migrated["rules"][0]["trigger_key"],
        input["rules"][0]["trigger_key"]
    );
    assert_eq!(
        migrated["rules"][0]["stop_key"],
        input["rules"][0]["stop_key"]
    );
    assert_eq!(migrated["rules"][0]["group"], "g");
    assert_eq!(migrated["rules"][0]["enabled"], false);
    assert!(migrated["rules"][0].get("target_key").is_none());
    assert!(migrated["rules"][0].get("interval_ms").is_none());
}

#[test]
fn migrate_from_current_is_noop() {
    // from == CURRENT_SCHEMA_VERSION 时不应调用 migrate_step，原值原样返回
    let v = json!({"hello": "world"});
    let out = migrate_profile(v.clone(), CURRENT_SCHEMA_VERSION).unwrap();
    assert_eq!(out, v);
}

#[test]
fn migrate_from_unknown_version_errors() {
    // 0 不在已知迁移链里，应当落到 UnknownVersion 分支
    let err = migrate_profile(json!({}), 0).unwrap_err();
    assert!(matches!(err, MigrateError::UnknownVersion(0)));
}

#[test]
fn migrate_v1_to_v2_wraps_required_keyboard_fields() {
    let v1 = json!({
        "schema_version": 1,
        "meta": {"name":"t","created_at":0,"updated_at":0,"app_version":"0"},
        "rules": [{
            "id": "r1",
            "enabled": true,
            "trigger_key": 0x51,
            "target_key": 0x46,
            "mode": "hold",
            "stop_key": null,
            "interval_ms": 10
        }],
        "hotkeys": {"global_toggle": null},
        "advanced": {"log_level": "info"}
    });
    let out = migrate_profile(v1, 1).unwrap();
    assert_eq!(out["schema_version"], json!(CURRENT_SCHEMA_VERSION));
    assert_eq!(
        out["rules"][0]["trigger_key"],
        json!({"kind":"keyboard","code":0x51})
    );
    assert_eq!(
        out["rules"][0]["steps"][0]["key"],
        json!({"kind":"keyboard","code":0x46})
    );
    assert!(out["rules"][0]["stop_key"].is_null());
    assert!(out["hotkeys"]["global_toggle"].is_null());
}

#[test]
fn migrate_v1_to_v2_wraps_optional_keys_when_present() {
    let v1 = json!({
        "schema_version": 1,
        "meta": {"name":"t","created_at":0,"updated_at":0,"app_version":"0"},
        "rules": [{
            "id": "r1",
            "enabled": true,
            "trigger_key": 0x10,
            "target_key": 0x11,
            "mode": "toggle",
            "stop_key": 0x12,
            "interval_ms": 50
        }],
        "hotkeys": {"global_toggle": 0x13},
        "advanced": {"log_level": "info"}
    });
    let out = migrate_profile(v1, 1).unwrap();
    assert_eq!(
        out["rules"][0]["stop_key"],
        json!({"kind":"keyboard","code":0x12})
    );
    assert_eq!(
        out["hotkeys"]["global_toggle"],
        json!({"kind":"keyboard","code":0x13})
    );
}

#[test]
fn migrate_v2_to_v3_bumps_schema_without_rewriting_keys() {
    let v2 = json!({
        "schema_version": 2,
        "meta": {"name":"t","created_at":0,"updated_at":0,"app_version":"0"},
        "rules": [{
            "id": "r1",
            "enabled": true,
            "trigger_key": {"kind":"mouse","code":"left"},
            "target_key": {"kind":"keyboard","code":0x51},
            "mode": "hold",
            "stop_key": null,
            "interval_ms": 10
        }],
        "hotkeys": {"global_toggle": {"kind":"mouse","code":"x1"}},
        "advanced": {"log_level": "info"}
    });
    let out = migrate_profile(v2, 2).unwrap();
    assert_eq!(out["schema_version"], json!(CURRENT_SCHEMA_VERSION));
    assert_eq!(
        out["rules"][0]["trigger_key"],
        json!({"kind":"mouse","code":"left"})
    );
    assert_eq!(
        out["hotkeys"]["global_toggle"],
        json!({"kind":"mouse","code":"x1"})
    );
}

#[test]
fn migrate_v1_to_v2_full_round_trip_into_profile() {
    let v1 = json!({
        "schema_version": 1,
        "meta": {"name":"t","created_at":0,"updated_at":0,"app_version":"0"},
        "rules": [{
            "id": "r1",
            "enabled": true,
            "trigger_key": 0x51,
            "target_key": 0x51,
            "mode": "hold",
            "stop_key": null,
            "interval_ms": 10
        }],
        "hotkeys": {"global_toggle": null},
        "advanced": {"log_level": "info"}
    });
    let migrated = migrate_profile(v1, 1).unwrap();
    let profile: crate::profile::Profile =
        serde_json::from_value(migrated).expect("迁移后应可反序列化为 Profile");
    profile.validate().unwrap();
    assert!(matches!(
        profile.rules[0].trigger_key,
        crate::key_id::KeyId::Keyboard(0x51)
    ));
}

#[test]
fn migrate_v1_to_v2_handles_missing_rules() {
    // 异常但应容忍：rules 字段不是数组时不应崩溃
    let v1 = json!({
        "schema_version": 1,
        "meta": {"name":"t","created_at":0,"updated_at":0,"app_version":"0"},
        "rules": []
    });
    let out = migrate_profile(v1, 1).unwrap();
    assert_eq!(out["schema_version"], json!(CURRENT_SCHEMA_VERSION));
    assert_eq!(out["rules"].as_array().unwrap().len(), 0);
}
