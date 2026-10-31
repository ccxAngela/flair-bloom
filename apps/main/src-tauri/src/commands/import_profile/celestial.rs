//! 衍天高手配置转换。结构依据上游 AppConfig / ConfigService / TaskSchedulerService。

use super::*;
use serde_json::Value;

pub(super) const SOURCE: &str = "衍天高手";

pub(super) fn detect(data: &str) -> bool {
    serde_json::from_str::<Value>(data).is_ok_and(|v| {
        v.get("Version").is_some_and(Value::is_number)
            && v.get("GlobalSwitch").is_some_and(Value::is_object)
            && (v.get("Schemes").is_some_and(Value::is_object)
                || v.get("Profiles").is_some_and(Value::is_array))
    })
}

pub(super) fn profiles(data: &str) -> Vec<(usize, String)> {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Vec::new();
    };
    value
        .get("Profiles")
        .and_then(Value::as_array)
        .map(|profiles| {
            profiles
                .iter()
                .enumerate()
                .filter_map(|(index, schemes)| {
                    let schemes = schemes.as_object()?;
                    if schemes.is_empty() {
                        return None;
                    }
                    let name = value
                        .get("ProfileNames")
                        .and_then(|v| v.get(index))
                        .and_then(Value::as_str)
                        .filter(|s| !s.trim().is_empty());
                    Some((
                        index,
                        name.map(str::to_string)
                            .unwrap_or_else(|| format!("方案 {}", index + 1)),
                    ))
                })
                .collect()
        })
        .unwrap_or_default()
}

pub(super) fn select_profile(data: &str, index: usize) -> Result<String, String> {
    let mut value: Value = serde_json::from_str(data).map_err(|e| format!("配置解析失败：{e}"))?;
    let selected = value
        .get("Profiles")
        .and_then(|v| v.get(index))
        .filter(|v| v.is_object())
        .cloned()
        .ok_or("该方案档位不存在")?;
    value["Schemes"] = selected;
    value["ActiveProfile"] = index.into();
    // 后续转换只读选中的镜像，避免重复合并同一档位。
    value
        .as_object_mut()
        .ok_or("配置根节点无效")?
        .remove("Profiles");
    serde_json::to_string(&value).map_err(|e| e.to_string())
}

fn keyboard(vk: u64, extended: bool) -> Option<KeyId> {
    // 本项目尚无扩展 Enter 的独立标识，不能把小键盘 Enter 静默改成主键区 Enter。
    if !(1..=255).contains(&vk) || (extended && vk == 13) {
        return None;
    }
    let key = KeyId::Keyboard(vk as u32);
    qzh_profile::accepts(
        qzh_profile::KeySlot::TriggerTarget,
        key,
        qzh_profile::InjectCaps::default(),
    )
    .ok()?;
    Some(key)
}

fn mouse(name: &str) -> Option<KeyId> {
    Some(KeyId::Mouse(match name {
        "Left" => MouseButton::Left,
        "Right" => MouseButton::Right,
        "Middle" => MouseButton::Middle,
        "XButton1" => MouseButton::X1,
        "XButton2" => MouseButton::X2,
        "WheelUp" => MouseButton::WheelUp,
        "WheelDown" => MouseButton::WheelDown,
        _ => return None,
    }))
}

fn source_key(raw: &str) -> Option<KeyId> {
    let parts: Vec<_> = raw.split(':').collect();
    match parts.as_slice() {
        ["K", vk, extended @ ("0" | "1")] => keyboard(vk.parse().ok()?, *extended == "1"),
        ["M", name] => mouse(name),
        _ => None,
    }
}

fn target_key(target: &Value) -> Option<KeyId> {
    match target
        .get("Kind")
        .and_then(Value::as_str)
        .unwrap_or("Keyboard")
    {
        "Keyboard" => keyboard(
            target.get("VirtualKey")?.as_u64()?,
            target
                .get("Extended")
                .and_then(Value::as_bool)
                .unwrap_or(false),
        ),
        "Mouse" => mouse(
            target
                .get("Mouse")
                .and_then(Value::as_str)
                .unwrap_or("Left"),
        ),
        "Wheel" => match target
            .get("Wheel")
            .and_then(Value::as_str)
            .unwrap_or("WheelUp")
        {
            "WheelUp" => mouse("WheelUp"),
            "WheelDown" => mouse("WheelDown"),
            _ => None,
        },
        _ => None,
    }
}

struct Converted {
    rules: Vec<BurstRule>,
    skipped: usize,
    warnings: Vec<String>,
}

fn convert(data: &str) -> Converted {
    let mut result = Converted {
        rules: Vec::new(),
        skipped: 0,
        warnings: Vec::new(),
    };
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return result;
    };
    let active = value
        .get("ActiveProfile")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let schemes = value
        .get("Profiles")
        .and_then(|v| v.get(active))
        .or_else(|| value.get("Schemes"))
        .and_then(Value::as_object);
    let Some(schemes) = schemes else {
        return result;
    };
    for (raw_source, scheme) in schemes {
        let targets = scheme.get("Targets").and_then(Value::as_array);
        let section = scheme
            .get("Section")
            .and_then(Value::as_str)
            .unwrap_or("Normal");
        let unsupported =
            !matches!(section, "Normal" | "Rotate") || targets.is_none_or(|t| t.len() != 1);
        let Some(target) = targets.and_then(|t| t.first()) else {
            result.skipped += 1;
            continue;
        };
        let modifiers = ["ModCtrl", "ModShift", "ModAlt"]
            .iter()
            .any(|k| target.get(k).and_then(Value::as_bool) == Some(true));
        let keys = source_key(raw_source).zip(target_key(target));
        let mode = match target
            .get("Mode")
            .and_then(Value::as_str)
            .unwrap_or("Toggle")
        {
            "Hold" => Some(BurstMode::Hold),
            "Toggle" => Some(BurstMode::Toggle),
            _ => None,
        };
        let supported = (!unsupported && !modifiers)
            .then_some(())
            .and(keys.zip(mode));
        let Some(((trigger_key, target_key), mode)) = supported else {
            result.skipped += 1;
            result.warnings.push(format!("跳过 {raw_source}：双宏、多目标轮流执行、修饰键组合或无法映射的按键／模式暂不支持。"));
            continue;
        };
        if result.rules.len() >= MAX_RULES {
            result.skipped += 1;
            continue;
        }
        // 上游 IntervalMs 是抬起后的间隔，这里的 interval_ms 是完整周期。
        let hold = target
            .get("HoldMs")
            .and_then(Value::as_u64)
            .filter(|n| *n > 0)
            .unwrap_or(26)
            .clamp(5, 100);
        let gap = target
            .get("IntervalMs")
            .and_then(Value::as_u64)
            .unwrap_or(26)
            .clamp(5, 90_000);
        let period = hold.saturating_add(gap);
        if period > u64::from(MAX_INTERVAL_MS) {
            result.warnings.push(format!(
                "{raw_source} 的周期超过本软件上限，已调整为 {MAX_INTERVAL_MS} ms。"
            ));
        }
        let group = (mode == BurstMode::Toggle && section == "Rotate").then(|| "衍天轮转".into());
        result.rules.push(BurstRule {
            id: make_id(),
            enabled: scheme
                .get("Enabled")
                .and_then(Value::as_bool)
                .unwrap_or(true)
                && target
                    .get("Enabled")
                    .and_then(Value::as_bool)
                    .unwrap_or(true),
            trigger_key,
            target_key,
            mode,
            stop_key: None,
            interval_ms: period.clamp(
                u64::from(MIN_EFFECTIVE_INTERVAL_MS),
                u64::from(MAX_INTERVAL_MS),
            ) as u32,
            group,
        });
    }
    result.warnings.insert(0, "保留完整连发周期（按压时长＋抬起间隔）；按压时长由气质花调度器决定，不能完全复现原按压时长。".into());
    result.warnings.push(
        "仅转换连发规则和全局启停键；外观、语音、驱动选择、方案切换热键及窗口绑定不导入。".into(),
    );
    result
}

pub(super) fn parse_rules(data: &str) -> Vec<BurstRule> {
    convert(data).rules
}

pub(super) fn parse_hotkeys(data: &str) -> Hotkeys {
    let Ok(value) = serde_json::from_str::<Value>(data) else {
        return Hotkeys::default();
    };
    let hotkey = |field: &str, default: Option<u32>| {
        let config = value.get(field);
        let has_key = config
            .and_then(|v| v.get("HasKey"))
            .and_then(Value::as_bool)
            .unwrap_or(default.is_some());
        let vk = config
            .and_then(|v| v.get("VirtualKey"))
            .and_then(Value::as_u64)
            .or(default.map(u64::from))?;
        if !has_key || !(1..=255).contains(&vk) {
            return None;
        }
        let key = KeyId::Keyboard(vk as u32);
        qzh_profile::accepts(
            qzh_profile::KeySlot::Hotkey,
            key,
            qzh_profile::InjectCaps::default(),
        )
        .ok()?;
        Some(key)
    };
    let toggle = hotkey("GlobalSwitch", Some(0x78));
    Hotkeys {
        global_toggle: toggle,
        global_stop: hotkey("IndependentClose", None).filter(|key| Some(*key) != toggle),
        panel_toggle: None,
    }
}

pub(super) fn parse_preview(data: &str, dir_name: &str) -> ImportPreview {
    let converted = convert(data);
    let hotkeys = parse_hotkeys(data);
    let min = converted
        .rules
        .iter()
        .map(|r| r.interval_ms)
        .min()
        .unwrap_or(0);
    let max = converted
        .rules
        .iter()
        .map(|r| r.interval_ms)
        .max()
        .unwrap_or(0);
    ImportPreview {
        source_app: SOURCE.into(),
        suggested_name: dir_name.into(),
        rule_count: converted.rules.len(),
        skipped_count: converted.skipped,
        interval_ms: min,
        interval_max_ms: max,
        warnings: converted.warnings,
        global_toggle: hotkeys.global_toggle,
        global_stop: hotkeys.global_stop,
        panel_toggle: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn converts_active_profile_without_merging_legacy_mirror() {
        let data = json!({"Version":20,"GlobalSwitch":{"HasKey":true,"VirtualKey":120},"IndependentClose":{"HasKey":true,"VirtualKey":121},"ActiveProfile":1,"Schemes":{"K:65:0":{"Targets":[{"VirtualKey":65}]}},"Profiles":[{}, {"M:XButton1":{"Section":"Rotate","Targets":[{"Kind":"Wheel","Wheel":"WheelDown","Mode":"Toggle","HoldMs":26,"IntervalMs":26}]}}],"ProfileNames":["","采集"]}).to_string();
        assert!(detect(&data));
        assert_eq!(profiles(&data), vec![(1, "采集".into())]);
        let preview = parse_preview(&data, "采集");
        assert_eq!(preview.rule_count, 1);
        assert_eq!(preview.interval_ms, 52);
        assert_eq!(preview.global_stop, Some(KeyId::Keyboard(121)));
        let rules = parse_rules(&data);
        assert_eq!(rules[0].trigger_key, KeyId::Mouse(MouseButton::X1));
        assert_eq!(rules[0].target_key, KeyId::Mouse(MouseButton::WheelDown));
        assert_eq!(rules[0].group.as_deref(), Some("衍天轮转"));
    }

    #[test]
    fn unsupported_actions_are_skipped_with_warning_and_disabled_rules_stay_disabled() {
        let data = json!({"Schemes":{
            "K:65:0":{"Targets":[{"VirtualKey":66,"ModCtrl":true}]},
            "K:66:0":{"Section":"Dual","Targets":[{"VirtualKey":66},{"VirtualKey":67}]},
            "K:13:1":{"Targets":[{"VirtualKey":65}]},
            "K:67:0":{"Enabled":false,"Targets":[{"VirtualKey":68,"Mode":"Hold"}]}
        }})
        .to_string();
        let preview = parse_preview(&data, "test");
        assert_eq!(preview.skipped_count, 3);
        let rules = parse_rules(&data);
        assert_eq!(rules.len(), 1);
        assert!(!rules[0].enabled);
        assert_eq!(rules[0].mode, BurstMode::Hold);
    }

    #[test]
    fn explicit_profile_selection_never_imports_other_slots() {
        let data = json!({"Profiles":[{"K:65:0":{"Targets":[{"VirtualKey":66}]}},{"K:67:0":{"Targets":[{"VirtualKey":68}]}}],"ActiveProfile":1}).to_string();
        let selected = select_profile(&data, 0).unwrap();
        let rules = parse_rules(&selected);
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].trigger_key, KeyId::Keyboard(65));
        assert!(select_profile(&data, 8).is_err());
    }
}
