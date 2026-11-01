//! Interception 就绪判定与只读诊断证据。

#[derive(Debug)]
pub struct InterceptionReadiness {
    pub runtime_available: bool,
    pub issue: Option<String>,
    pub warning: Option<String>,
    pub detail: String,
}

pub fn assess_readiness(
    services: bool,
    slots: Option<&[(i32, String)]>,
    classes: Result<(Vec<u32>, Vec<u32>), String>,
) -> InterceptionReadiness {
    let keyboards: Vec<_> = slots
        .unwrap_or_default()
        .iter()
        .filter_map(|(id, _)| (1..=10).contains(id).then_some(*id))
        .collect();
    let mice: Vec<_> = slots
        .unwrap_or_default()
        .iter()
        .filter_map(|(id, _)| (11..=20).contains(id).then_some(*id))
        .collect();
    let mut detail = format!(
        "Interception 服务已注册：{services}；运行通道可打开：{}；可读键盘 {}/10，槽号 {keyboards:?}；可读鼠标 {}/10，槽号 {mice:?}",
        slots.is_some(), keyboards.len(), mice.len()
    );
    let mut warnings = Vec::new();
    match classes {
        Ok((keyboard_classes, mouse_classes)) => {
            detail.push_str(&format!(
                "；Windows 类设备编号：键盘 {keyboard_classes:?}、鼠标 {mouse_classes:?}"
            ));
            if keyboard_classes
                .iter()
                .chain(&mouse_classes)
                .any(|id| *id >= 10)
            {
                warnings.push("Windows 类设备编号偏高，仅作容量风险参考，不代表当前可读的 Interception 设备不可用。".to_string());
            }
        }
        Err(error) => {
            warnings.push(format!("Windows 类设备编号采集失败：{error}；按实际驱动通道和可读键盘判定，不因此阻止启用。"));
        }
    }
    if keyboards.len() == 10 || mice.len() == 10 {
        // SDK 的最后一个槽仍是合法设备，容量不足只影响之后接入的设备。
        warnings.push("可读设备已占满某类的 10 个槽，现有可读设备仍可使用。仅凭计数无法区分恰好满槽与额外设备未分到槽，额外设备是否缺槽尚未确认。若新接入设备失效，请重启 Windows；拔插和退出应用不能回收驱动槽位。".into());
    }
    if !keyboards.is_empty() && mice.is_empty() {
        warnings.push("未读取到鼠标设备，键盘仍可使用游戏模式；鼠标注入可能回退通用通道，在游戏中不保证生效。".into());
    }
    let issue = if slots.is_none() {
        Some(if services {
            "游戏模式驱动已注册，但运行通道无法打开。首次安装后请重启 Windows；如已重启，请检查权限并退出其他使用 Interception 的程序后重试。".into()
        } else {
            "游戏模式驱动未安装，或通道被其他程序占用。请先检查驱动安装状态。".into()
        })
    } else if keyboards.is_empty() {
        Some("游戏模式通道可打开，但未读取到可用键盘。请在诊断修复中检查键盘连接、驱动状态和设备槽；首次安装后需重启 Windows。".into())
    } else {
        None
    };
    let warning = (!warnings.is_empty()).then(|| warnings.join("\n"));
    detail.push_str(if issue.is_some() {
        "；启用判定：阻止"
    } else {
        "；启用判定：允许"
    });
    if let Some(reason) = &issue {
        detail.push_str(&format!("；阻止原因：{reason}"));
    }
    if let Some(warning) = &warning {
        detail.push_str(&format!("；诊断提示：{warning}"));
    }
    InterceptionReadiness {
        runtime_available: slots.is_some(),
        issue,
        warning,
        detail,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(keyboards: i32, mice: i32) -> Vec<(i32, String)> {
        (1..=keyboards)
            .chain(11..11 + mice)
            .map(|id| (id, "device".into()))
            .collect()
    }

    #[test]
    fn tenth_keyboard_and_mouse_slots_remain_usable() {
        for (keyboards, mice) in [(10, 1), (1, 10), (10, 10)] {
            let slots = slots(keyboards, mice);
            let health = assess_readiness(true, Some(&slots), Ok((vec![9], vec![9])));
            assert!(health.issue.is_none(), "{health:?}");
            let warning = health.warning.unwrap();
            assert!(warning.contains("现有可读设备仍可使用"));
            assert!(warning.contains("额外设备是否缺槽尚未确认"));
        }
    }

    #[test]
    fn readable_last_slots_are_valid_even_with_high_windows_indices() {
        let devices = vec![(10, "keyboard".into()), (20, "mouse".into())];
        let health = assess_readiness(true, Some(&devices), Ok((vec![10], vec![20])));
        assert!(health.issue.is_none(), "{health:?}");
        assert!(health.detail.contains("槽号 [10]"));
        assert!(health.detail.contains("槽号 [20]"));
    }

    #[test]
    fn high_class_numbers_do_not_disable_readable_devices() {
        let slots = vec![(4, "keyboard".into()), (13, "mouse".into())];
        let health = assess_readiness(true, Some(&slots), Ok((vec![0, 12], vec![11])));
        assert!(health.issue.is_none(), "{health:?}");
        assert!(health.warning.unwrap().contains("编号偏高"));
        assert!(health.detail.contains("槽号 [4]"));
        assert!(health.detail.contains("键盘 [0, 12]、鼠标 [11]"));
    }

    #[test]
    fn keyboard_only_backend_remains_available() {
        let slots = slots(1, 0);
        let health = assess_readiness(true, Some(&slots), Ok((vec![0], vec![])));
        assert!(health.issue.is_none(), "{health:?}");
        assert!(health.warning.unwrap().contains("鼠标注入可能回退"));
    }

    #[test]
    fn failed_class_read_is_diagnostic_only() {
        let slots = slots(1, 1);
        let health = assess_readiness(true, Some(&slots), Err("拒绝访问".into()));
        assert!(health.issue.is_none(), "{health:?}");
        assert!(health.detail.contains("拒绝访问"));
        assert!(health.warning.is_some());
    }

    #[test]
    fn readable_runtime_wins_over_missing_service_registration() {
        let slots = slots(1, 1);
        let health = assess_readiness(false, Some(&slots), Ok((vec![], vec![])));
        assert!(health.runtime_available);
        assert!(health.issue.is_none());
        assert!(health.warning.is_none());
    }

    #[test]
    fn unavailable_context_is_blocked_with_or_without_service() {
        for services in [false, true] {
            let health = assess_readiness(services, None, Ok((vec![0], vec![0])));
            assert!(!health.runtime_available);
            assert!(health.issue.is_some());
            assert!(health.detail.contains("启用判定：阻止"));
            assert!(health.detail.contains("Windows 类设备编号"));
        }
    }

    #[test]
    fn empty_or_mouse_only_runtime_has_no_usable_keyboard() {
        for devices in [
            vec![],
            vec![(11, "mouse".into())],
            vec![(0, "invalid".into()), (21, "invalid".into())],
        ] {
            let health = assess_readiness(true, Some(&devices), Ok((vec![0], vec![0])));
            assert!(health.issue.unwrap().contains("未读取到可用键盘"));
            assert!(!health
                .warning
                .as_deref()
                .is_some_and(|warning| warning.contains("键盘仍可使用")));
        }
    }
}
