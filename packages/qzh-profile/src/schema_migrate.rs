//! [`crate::profile::Profile`] 的 schema 迁移入口。
//!
//! 调度通用 [`migrate::run_migrations`] 把 JSON 形式的旧版 Profile 升级到当前版本。
//! 每个版本对应的具体改写规则集中在 `migrate_step` 内。

use migrate::{run_migrations, MigrateError};
use serde_json::{json, Value};

use crate::profile::CURRENT_SCHEMA_VERSION;

/// 把 `data`（JSON 形式的 [`crate::profile::Profile`]）从 `from` 版本迁移到当前版本。
///
/// 内部委托给通用 [`migrate::run_migrations`] 调度，每一步由模块内的 `migrate_step` 处理。
pub fn migrate_profile(data: Value, from: u32) -> Result<Value, MigrateError> {
    run_migrations(data, from, CURRENT_SCHEMA_VERSION, migrate_step)
}

fn migrate_step(data: Value, from_version: u32) -> Result<Value, MigrateError> {
    match from_version {
        1 => migrate_v1_to_v2(data),
        2 => migrate_v2_to_v3(data),
        3 => migrate_v3_to_v4(data),
        4 => migrate_v4_to_v5(data),
        v => Err(MigrateError::UnknownVersion(v)),
    }
}

/// v1 → v2：所有按键字段从裸 `u32` 改为 [`crate::key_id::KeyId`]。
///
/// 涉及位置：
/// - `rules[].trigger_key` / `rules[].target_key`：必填，`u32` → `Keyboard(u32)`。
/// - `rules[].stop_key`：可选，缺省/`null` 保留；存在则同样包装。
/// - `hotkeys.global_toggle`：可选，缺省/`null` 保留；存在则包装。
fn migrate_v1_to_v2(mut data: Value) -> Result<Value, MigrateError> {
    if let Some(rules) = data.get_mut("rules").and_then(|r| r.as_array_mut()) {
        for rule in rules.iter_mut() {
            wrap_keyboard_field(rule, "trigger_key");
            wrap_keyboard_field(rule, "target_key");
            wrap_optional_keyboard_field(rule, "stop_key");
        }
    }
    if let Some(hotkeys) = data.get_mut("hotkeys") {
        wrap_optional_keyboard_field(hotkeys, "global_toggle");
    }
    data["schema_version"] = json!(2);
    Ok(data)
}

/// 必填字段：若存在且为整数，包装为 `{kind: "keyboard", code: <vk>}`。
/// 已经是对象（理论上不该出现在 v1）则原样保留，避免重复包装。
fn wrap_keyboard_field(obj: &mut Value, field: &str) {
    let Some(val) = obj.get_mut(field) else {
        return;
    };
    if let Some(vk) = val.as_u64() {
        *val = json!({ "kind": "keyboard", "code": vk });
    }
}

/// 可选字段：`null` / 缺省保持不变；整数包装；其他类型不动。
fn wrap_optional_keyboard_field(obj: &mut Value, field: &str) {
    let Some(val) = obj.get_mut(field) else {
        return;
    };
    if val.is_null() {
        return;
    }
    if let Some(vk) = val.as_u64() {
        *val = json!({ "kind": "keyboard", "code": vk });
    }
}

/// v2 → v3：新增 WheelUp / WheelDown 枚举值，旧数据无结构变更，仅升版本号。
fn migrate_v2_to_v3(mut data: Value) -> Result<Value, MigrateError> {
    data["schema_version"] = json!(3);
    Ok(data)
}

/// v3 → v4：BurstRule 新增可选 `group` 字段，旧数据无结构变更（`#[serde(default)]` 兜底），仅升版本号。
fn migrate_v3_to_v4(mut data: Value) -> Result<Value, MigrateError> {
    data["schema_version"] = json!(4);
    Ok(data)
}

fn migrate_v4_to_v5(mut data: Value) -> Result<Value, MigrateError> {
    if let Some(rules) = data.get_mut("rules").and_then(Value::as_array_mut) {
        for rule in rules {
            let Some(fields) = rule.as_object_mut() else {
                continue;
            };
            if !fields.contains_key("steps") {
                if let Some((key, interval)) =
                    fields.get("target_key").zip(fields.get("interval_ms"))
                {
                    let steps = json!([{ "key": key, "interval_ms": interval }]);
                    fields.remove("target_key");
                    fields.remove("interval_ms");
                    fields.insert("steps".into(), steps);
                }
            }
        }
    }
    data["schema_version"] = json!(5);
    Ok(data)
}

#[cfg(test)]
#[path = "schema_migrate_tests.rs"]
mod tests;
