//! 极简按键 Config.ini 的按键与设置迁移；顺序循环转换为默认停用的长按规则。

use super::*;
use std::collections::HashMap;

fn fields(data: &str) -> HashMap<String, String> {
    let mut in_config = false;
    let mut fields = HashMap::new();
    for line in data.lines().map(str::trim) {
        if line.starts_with('[') {
            in_config = line.eq_ignore_ascii_case("[Config]");
            continue;
        }
        if !in_config || line.starts_with(['#', ';']) {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            fields.insert(key.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    fields
}

pub(super) fn detect(data: &str) -> bool {
    let fields = fields(data);
    ["start", "stop", "press", "enabled", "interval"]
        .iter()
        .all(|key| fields.contains_key(*key))
}

fn key(name: &str) -> Option<KeyId> {
    let name = name.trim().to_ascii_uppercase();
    let vk = match name.as_str() {
        "NONE" | "" => return None,
        "BACK" => 8,
        "TAB" => 9,
        "ENTER" => 13,
        "PAUSE" => 19,
        "CAPSLOCK" => 20,
        "ESC" => 27,
        "SPACE" => 32,
        "PAGEUP" => 33,
        "PAGEDOWN" => 34,
        "END" => 35,
        "HOME" => 36,
        "LEFT" => 37,
        "UP" => 38,
        "RIGHT" => 39,
        "DOWN" => 40,
        "PRINTSCN" => 44,
        "INSERT" => 45,
        "DELETE" => 46,
        "LWIN" => 91,
        "RWIN" => 92,
        "SCROLL" => 145,
        "LSHIFT" => 160,
        "RSHIFT" => 161,
        "LCTRL" => 162,
        "RCTRL" => 163,
        "LALT" => 164,
        "RALT" => 165,
        "OEM_1" => 186,
        "OEM_PLUS" => 187,
        "OEM_COMMA" => 188,
        "OEM_MINUS" => 189,
        "OEM_PERIOD" => 190,
        "OEM_2" => 191,
        "OEM_3" => 192,
        "OEM_4" => 219,
        "OEM_5" => 220,
        "OEM_6" => 221,
        "OEM_7" => 222,
        _ if name.len() == 1 && name.as_bytes()[0].is_ascii_uppercase() => {
            u32::from(name.as_bytes()[0])
        }
        _ if name.starts_with("NUM") && name.len() == 4 => {
            48 + name[3..].parse::<u32>().ok().filter(|n| *n <= 9)?
        }
        _ if name.starts_with('F') => {
            111 + name[1..]
                .parse::<u32>()
                .ok()
                .filter(|n| (1..=24).contains(n))?
        }
        _ => return None,
    };
    Some(KeyId::Keyboard(vk))
}

pub(super) fn parse_rules(data: &str) -> Vec<BurstRule> {
    let fields = fields(data);
    let interval = fields
        .get("interval")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(50)
        .clamp(
            u64::from(MIN_EFFECTIVE_INTERVAL_MS),
            u64::from(MAX_INTERVAL_MS),
        ) as u32;
    fields
        .get("press")
        .into_iter()
        .flat_map(|v| v.split(','))
        .filter_map(key)
        .filter(|key| {
            qzh_profile::accepts(
                qzh_profile::KeySlot::TriggerTarget,
                *key,
                qzh_profile::InjectCaps::default(),
            )
            .is_ok()
        })
        .take(MAX_RULES)
        .map(|key| BurstRule {
            id: make_id(),
            enabled: false,
            trigger_key: key,
            steps: vec![qzh_profile::profile::OutputStep {
                key,
                interval_ms: interval,
            }],
            mode: BurstMode::Hold,
            stop_key: None,
            group: None,
        })
        .collect()
}

pub(super) fn parse_hotkeys(data: &str) -> Hotkeys {
    let fields = fields(data);
    let start = fields.get("start").and_then(|v| key(v));
    Hotkeys {
        global_toggle: start,
        global_stop: fields
            .get("stop")
            .and_then(|v| key(v))
            .filter(|key| Some(*key) != start),
        panel_toggle: None,
    }
}

pub(super) fn parse_preview(data: &str, name: &str) -> ImportPreview {
    let fields = fields(data);
    let rules = parse_rules(data);
    let count = fields
        .get("press")
        .map(|v| v.split(',').filter(|s| !s.trim().is_empty()).count())
        .unwrap_or(0);
    let hotkeys = parse_hotkeys(data);
    let interval = rules.first().map(|r| r.steps[0].interval_ms).unwrap_or(0);
    ImportPreview { source_app: "极简按键".into(), suggested_name: name.into(), rule_count: rules.len(), skipped_count: count.saturating_sub(rules.len()), interval_ms: interval, interval_max_ms: interval,
        warnings: vec!["此格式为顺序循环配置，按键列表迁移为默认停用的同键长按规则；导入后请确认触发方式、间隔并自行启用，不能复现原自动顺序执行。".into(), "迁移启动／停止键，不导入暂停键、声音、驱动、协议记录；NUM0～NUM9 是原格式的主键区数字键。".into()],
        global_toggle: hotkeys.global_toggle, global_stop: hotkeys.global_stop, panel_toggle: None }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn imports_real_ini_shape_without_confusing_num3_with_numpad3() {
        let data = "# synthetic fixture\n[Config]\nStart=F8\nStop=F8\nPause=NONE\nVideo=0\nInterval=25\nEnabled=0,1\nPress=Q,NUM3\n";
        assert!(detect(data));
        let rules = parse_rules(data);
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[1].steps[0].key, KeyId::Keyboard(51));
        assert!(rules.iter().all(|r| !r.enabled));
        assert_eq!(
            parse_hotkeys(data).global_toggle,
            Some(KeyId::Keyboard(119))
        );
        assert_eq!(parse_hotkeys(data).global_stop, None);
        assert!(!detect("[Config]\nStart=F1\nUnrelated=1"));
    }
}
