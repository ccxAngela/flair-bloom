//! 小叽按键格式，依据源码 ConfigStore / AppTypes / RepeaterEngine，配置位于 EXE 同级。

use super::*;
use serde_json::Value;

pub(super) fn detect(data: &str) -> bool {
    serde_json::from_str::<Value>(data).is_ok_and(|value| {
        value.get("profiles").is_some_and(Value::is_array)
            && value.get("activeProfileId").is_some_and(Value::is_number)
            && (value.get("driverMode").is_some() || value.get("beepEnabled").is_some())
    })
}

pub(super) fn profiles(data: &str) -> Vec<(usize, String)> {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    value
        .get("profiles")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
        .filter_map(|(index, profile)| {
            profile.as_object()?;
            Some((
                index,
                profile
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|s| !s.trim().is_empty())
                    .map(str::to_string)
                    .unwrap_or_else(|| format!("方案 {}", index + 1)),
            ))
        })
        .collect()
}

pub(super) fn select_profile(data: &str, index: usize) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(data).map_err(|e| e.to_string())?;
    let selected = value
        .get("profiles")
        .and_then(|p| p.get(index))
        .filter(|v| v.is_object())
        .cloned()
        .ok_or("小叽方案档位不存在")?;
    value["activeProfileId"] = selected.get("id").cloned().unwrap_or(0.into());
    value["profiles"] = vec![selected].into();
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

fn active_profile(value: &Value) -> Option<&Value> {
    let profiles = value.get("profiles")?.as_array()?;
    profiles
        .iter()
        .find(|p| p.get("id") == value.get("activeProfileId"))
        .or_else(|| profiles.first())
}

fn key(text: &str) -> Option<KeyId> {
    let (device, code) = text.split_once(':')?;
    let code = code.parse::<u32>().ok()?;
    match device {
        "K" if (1..=255).contains(&code) => Some(KeyId::Keyboard(code)),
        "M" if [1, 2, 4, 5, 6].contains(&code) => Some(vk_to_key_id(code)),
        _ => None,
    }
}

fn converted(data: &str) -> (Vec<BurstRule>, usize, Vec<String>) {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return (Vec::new(), 0, Vec::new());
    };
    let Some(profile) = active_profile(&value) else {
        return (Vec::new(), 0, Vec::new());
    };
    let mut rules = Vec::new();
    let mut skipped = 0;
    let mut warnings = vec!["只迁移按键连发和当前方案启停键；暂停键、前台窗口限制、WASD 防卡位、连发优先、声音和驱动选择不导入。".into(), "连发周期按原抬起间隔＋5 ms 按压宽度转换；实际按压时长由气质花决定。".into()];
    for binding in profile
        .get("bindings")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let key = binding
            .get("key")
            .and_then(Value::as_str)
            .and_then(key)
            .filter(|key| {
                qzh_profile::accepts(
                    qzh_profile::KeySlot::TriggerTarget,
                    *key,
                    qzh_profile::InjectCaps::default(),
                )
                .is_ok()
            });
        let raw_mode = binding
            .get("mode")
            .and_then(Value::as_str)
            .unwrap_or("toggle");
        let fixed = binding
            .get("clickPos")
            .and_then(|p| p.get("mode"))
            .and_then(Value::as_str)
            == Some("fixed");
        let delay = binding
            .get("delayMs")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0);
        if key.is_none()
            || delay.is_none()
            || fixed
            || !matches!(raw_mode, "hold" | "independent" | "toggle")
        {
            skipped += 1;
            warnings.push(format!(
                "跳过 {}：不支持持续按住、顺序切换、固定位置点击或无法映射的按键／间隔。",
                binding
                    .get("key")
                    .and_then(Value::as_str)
                    .unwrap_or("未知按键")
            ));
            continue;
        }
        let Some((key, delay)) = key.zip(delay) else {
            continue;
        };
        if rules.len() >= MAX_RULES {
            skipped += 1;
            continue;
        }
        let enabled = profile
            .get("enabled")
            .and_then(Value::as_bool)
            .unwrap_or(true)
            && binding
                .get("enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true)
            && raw_mode != "toggle";
        rules.push(BurstRule {
            id: make_id(),
            enabled,
            trigger_key: key,
            steps: vec![qzh_profile::profile::OutputStep {
                key,
                interval_ms: delay.saturating_add(5).clamp(
                    u64::from(MIN_EFFECTIVE_INTERVAL_MS),
                    u64::from(MAX_INTERVAL_MS),
                ) as u32,
            }],
            mode: if raw_mode == "hold" {
                BurstMode::Hold
            } else {
                BurstMode::Toggle
            },
            stop_key: None,
            group: (raw_mode == "independent").then(|| "小叽独立切换".into()),
        });
        if raw_mode == "toggle" {
            warnings.push("小叽的开关模式随总开关自动连发，已迁移为默认停用的同键切换规则；导入后需确认触发方式并启用，打开全局开关不会自动启动这些规则。".into());
        }
        if delay.saturating_add(5) > u64::from(MAX_INTERVAL_MS) {
            warnings.push(format!("周期超过上限的规则已钳为 {MAX_INTERVAL_MS} ms。"));
        }
    }
    let macros = value
        .get("macros")
        .or_else(|| profile.get("macros"))
        .and_then(Value::as_array)
        .map_or(0, Vec::len);
    if macros > 0 {
        skipped += macros;
        warnings.push(format!(
            "跳过 {macros} 份宏循环：本软件尚不支持多步骤宏与循环次数的等价转换。"
        ));
    }
    warnings.sort();
    warnings.dedup();
    (rules, skipped, warnings)
}

pub(super) fn parse_rules(data: &str) -> Vec<BurstRule> {
    converted(data).0
}

pub(super) fn parse_hotkeys(data: &str) -> Hotkeys {
    let toggle = serde_json::from_str::<Value>(data)
        .ok()
        .and_then(|value| {
            active_profile(&value)
                .and_then(|p| p.get("controlHotkey"))
                .and_then(Value::as_str)
                .and_then(key)
        })
        .filter(|key| {
            qzh_profile::accepts(
                qzh_profile::KeySlot::Hotkey,
                *key,
                qzh_profile::InjectCaps::default(),
            )
            .is_ok()
        });
    Hotkeys {
        global_toggle: toggle,
        global_stop: None,
        panel_toggle: None,
    }
}

pub(super) fn parse_preview(data: &str, name: &str) -> ImportPreview {
    let (rules, skipped, warnings) = converted(data);
    ImportPreview {
        source_app: "小叽按键".into(),
        suggested_name: name.into(),
        rule_count: rules.len(),
        skipped_count: skipped,
        interval_ms: rules
            .iter()
            .map(|r| r.steps[0].interval_ms)
            .min()
            .unwrap_or(0),
        interval_max_ms: rules
            .iter()
            .map(|r| r.steps[0].interval_ms)
            .max()
            .unwrap_or(0),
        warnings,
        global_toggle: parse_hotkeys(data).global_toggle,
        global_stop: None,
        panel_toggle: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_profile_id_modes_keys_and_pulse_width_from_source_schema() {
        let data = json!({"driverMode":"legacy","activeProfileId":9,"profiles":[{"id":1,"name":"一","bindings":[]},{"id":9,"name":"二","controlHotkey":"K:120","bindings":[
            {"key":"K:65","mode":"hold","delayMs":30}, {"key":"M:5","mode":"independent","delayMs":10}, {"key":"K:67","mode":"toggle","delayMs":20}
        ]}]}).to_string();
        assert!(detect(&data));
        assert_eq!(profiles(&data)[1], (1, "二".into()));
        let rules = parse_rules(&data);
        assert_eq!(rules.len(), 3);
        assert_eq!(rules[0].steps[0].interval_ms, 35);
        assert_eq!(rules[0].mode, BurstMode::Hold);
        assert_eq!(rules[1].steps[0].key, KeyId::Mouse(MouseButton::X1));
        assert!(rules[1].group.is_some());
        assert!(!rules[2].enabled);
        assert_eq!(
            parse_hotkeys(&data).global_toggle,
            Some(KeyId::Keyboard(120))
        );
        assert!(parse_rules(&select_profile(&data, 0).unwrap()).is_empty());
    }

    #[test]
    fn skips_actions_without_an_equivalent_instead_of_enabling_them() {
        let data = json!({"activeProfileId":1,"profiles":[{"id":1,"bindings":[{"key":"K:65","mode":"longPress","delayMs":10},{"key":"M:1","mode":"hold","delayMs":10,"clickPos":{"mode":"fixed"}}]}],"macros":[{"steps":[]}]}).to_string();
        let preview = parse_preview(&data, "test");
        assert_eq!(preview.rule_count, 0);
        assert_eq!(preview.skipped_count, 3);
        assert!(!preview.warnings.is_empty());
    }
}
