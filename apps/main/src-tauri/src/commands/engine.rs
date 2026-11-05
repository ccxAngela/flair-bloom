//! 规则 CRUD + 输入模式切换 + 按键捕获。驱动管理已迁至 [`super::driver`]。

use crate::engine::{BurstEngine, RuleStates};
use qzh_profile::key_policy::{slot_policy, KeySlot, SlotPolicy};
#[cfg(windows)]
use qzh_profile::{find_rule_violations, KeyRejection};
use qzh_profile::{BurstRule, Hotkeys, InjectCaps, KeyId, MAX_RULES};
use serde::Serialize;
use std::sync::{atomic::Ordering, Arc};
#[allow(unused_imports)]
use tauri::{AppHandle, Manager, State};
use win_input::try_consume_relay_injection;

pub struct EngineState(pub Arc<BurstEngine>);

#[tauri::command]
pub fn preview_output(
    state: State<EngineState>,
    steps: Vec<qzh_profile::OutputStep>,
) -> Result<String, String> {
    let caps = current_inject_caps();
    for step in &steps {
        qzh_profile::accepts(KeySlot::Target, step.key, caps)
            .map_err(|_| "队列包含当前模式不支持的输出键".to_string())?;
    }
    state.0.preview_output(steps)
}

#[tauri::command]
pub fn stop_preview(state: State<EngineState>, rule_id: String) -> Result<(), String> {
    state.0.stop_preview(&rule_id)
}

#[derive(Serialize)]
pub struct RecordingDto {
    session_id: u64,
    recording: bool,
    steps: Vec<qzh_profile::OutputStep>,
    notice: Option<String>,
}

impl From<burst_engine::RecordingSnapshot> for RecordingDto {
    fn from(value: burst_engine::RecordingSnapshot) -> Self {
        Self {
            session_id: value.session_id,
            recording: value.recording,
            steps: value.steps,
            notice: value.notice,
        }
    }
}

#[tauri::command]
pub fn start_recording(state: State<EngineState>, stop_key: KeyId) -> Result<RecordingDto, String> {
    if !matches!(stop_key, KeyId::Keyboard(vk) if !qzh_profile::key_policy::is_modifier_vk(vk)) {
        return Err("结束录制键需要使用普通键盘按键".into());
    }
    let hotkeys = state.0.get_hotkeys();
    if [
        hotkeys.global_toggle,
        hotkeys.global_stop,
        hotkeys.panel_toggle,
    ]
    .contains(&Some(stop_key))
    {
        return Err("结束录制键不能与全局热键重合".into());
    }
    state.0.start_recording(stop_key).map(Into::into)
}

#[tauri::command]
pub fn get_recording(state: State<EngineState>, session_id: u64) -> Result<RecordingDto, String> {
    let snapshot = state.0.get_recording();
    if snapshot.session_id != session_id {
        return Err("录制会话已失效".into());
    }
    Ok(snapshot.into())
}

#[tauri::command]
pub fn stop_recording(state: State<EngineState>, session_id: u64) -> Result<RecordingDto, String> {
    state.0.stop_recording(session_id).map(Into::into)
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct RelayKeyResult {
    pub accepted_physical: bool,
    pub handled: bool,
}

/// 面板、浮窗与自动开启共用，引擎统一排队通知两个窗口，避免旁路广播造成乱序。
#[tauri::command]
pub fn set_global_enabled(state: State<EngineState>, enabled: bool) -> Result<(), String> {
    if enabled {
        if let Some(error) = state.0.output_error() {
            return Err(error);
        }
    }
    state.0.set_global_enabled(enabled, true);
    Ok(())
}

/// 运行时更新全局热键（不写盘，写盘由 `save_profile` 负责）。
///
/// 与录入界面、配置文件加载共用 [`qzh_profile::key_policy`] 的判定：前端拦不住导入的
/// `.qzh` 与手工编辑，最终把关落在这里。
#[tauri::command]
pub fn set_global_hotkeys(state: State<EngineState>, mut hotkeys: Hotkeys) -> Result<(), String> {
    let dropped = qzh_profile::key_policy::sanitize_hotkeys(&mut hotkeys);
    if !dropped.is_empty() {
        return Err("全局热键只能绑定键盘按键（含左右修饰键）".to_string());
    }
    state.0.set_hotkeys(hotkeys);
    Ok(())
}

/// 五个录入槽位在当前输入模式下的允许集，供前端按键捕获组件查表。
///
/// 前端不再自行维护白名单：能力（后端注入得了什么）随输入模式变，策略（产品上让不让绑）
/// 写在 [`qzh_profile::key_policy`]，两者都只有这一份。输入模式切换后需要重新拉取。
#[derive(Debug, Clone, Serialize)]
pub struct KeyPolicies {
    /// 三个全局热键槽。纯读。
    pub hotkey: SlotPolicy,
    /// 规则的启动键 / 停止键，且与连发按键不同。纯读。
    pub trigger: SlotPolicy,
    /// 规则的连发按键，且与启动 / 停止键不同。纯写。
    pub target: SlotPolicy,
    /// 长按连发里启动键与连发按键重合（默认模式、横版单键）。读写取交集。
    pub trigger_target: SlotPolicy,
    /// 切换连发里启动键与连发按键重合。后端不支持重合态时是空集，键盘键也不收。
    pub trigger_target_toggle: SlotPolicy,
    /// 当前后端能否支持 Toggle 规则的重合态。为 false 时默认模式建不出切换连发。
    pub coincident_toggle: bool,
}

#[tauri::command]
pub fn get_key_policy() -> KeyPolicies {
    let caps = current_inject_caps();
    KeyPolicies {
        hotkey: slot_policy(KeySlot::Hotkey, caps),
        trigger: slot_policy(KeySlot::Trigger, caps),
        target: slot_policy(KeySlot::Target, caps),
        trigger_target: slot_policy(KeySlot::TriggerTarget, caps),
        trigger_target_toggle: slot_policy(KeySlot::TriggerTargetToggle, caps),
        coincident_toggle: caps.coincident_toggle,
    }
}

/// 当前输入后端的注入能力。非 Windows 无后端概念，按最宽松处理。
pub(crate) fn current_inject_caps() -> InjectCaps {
    #[cfg(windows)]
    {
        caps_for_mode(win_input::current_mode())
    }
    #[cfg(not(windows))]
    {
        InjectCaps::default()
    }
}

#[cfg(windows)]
pub(crate) fn caps_for_mode(mode: win_input::InputMode) -> InjectCaps {
    InjectCaps {
        coincident_toggle: !mode.requires_distinct_target_for_toggle(),
    }
}

/// 拒绝原因的中文说明。文案留在应用层，`qzh-profile` 只给机器可读的枚举。
#[cfg(windows)]
pub(crate) fn rejection_message(reason: KeyRejection, mode_label: &str) -> String {
    match reason {
        KeyRejection::ModifierNotAllowed => {
            "的按键是修饰键。修饰键只能绑定全局热键，请换其它按键。".to_string()
        }
        KeyRejection::MouseNotAllowed => "的按键不支持鼠标按键。".to_string(),
        KeyRejection::CoincidentToggleUnsupported => format!(
            "是切换连发，且启动 / 停止键与连发按键相同，{mode_label} 模式不支持。请在高级设置里把连发按键改成另一个键。"
        ),
    }
}

#[tauri::command]
pub fn get_global_enabled(state: State<EngineState>) -> bool {
    state.0.global_enabled.load(Ordering::SeqCst)
}

#[tauri::command]
pub fn set_rules(state: State<EngineState>, rules: Vec<BurstRule>) -> Result<(), String> {
    if rules.len() > MAX_RULES {
        return Err(format!("规则数量 {} 超过上限 {}", rules.len(), MAX_RULES));
    }
    for (i, rule) in rules.iter().enumerate() {
        rule.validate()
            .map_err(|error| format!("第 {} 条规则：{error}", i + 1))?;
    }

    #[cfg(windows)]
    check_rules_for_mode(&rules, win_input::current_mode())?;

    state.0.set_rules(rules);
    Ok(())
}

#[tauri::command]
pub fn get_rules(state: State<EngineState>) -> Vec<BurstRule> {
    state.0.get_rules()
}

#[tauri::command]
pub fn get_hotkeys(state: State<EngineState>) -> Hotkeys {
    state.0.get_hotkeys()
}

/// 保留注册以维持 D9 语义兼容（running ∪ paused）；面板与浮窗已改用 [`get_rule_states`]。
#[tauri::command]
pub fn get_active_rules(state: State<EngineState>) -> Vec<String> {
    state.0.get_active_ids()
}

/// 活跃规则的运行 / 暂停分区。引擎不依赖 serde，在命令层映射成可序列化结构。
#[derive(Debug, Clone, Serialize)]
pub struct RuleStatesDto {
    pub running: Vec<String>,
    pub paused: Vec<String>,
    pub progress: Vec<ProgressDto>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProgressDto {
    pub rule_id: String,
    pub step_index: usize,
    pub step_count: usize,
    pub completed_cycles: u64,
    pub paused: bool,
    pub error: Option<String>,
    pub lateness_ms: u64,
    pub max_lateness_ms: u64,
}

impl From<RuleStates> for RuleStatesDto {
    fn from(s: RuleStates) -> Self {
        Self {
            running: s.running,
            paused: s.paused,
            progress: Vec::new(),
            error: s.error,
        }
    }
}

/// 前端每拍只调这一个：运行与暂停取自同一把锁下的快照，两次调用拼起来可能错拍。
#[tauri::command]
pub fn get_rule_states(state: State<EngineState>) -> RuleStatesDto {
    let mut states: RuleStatesDto = state.0.get_rule_states().into();
    states.progress = state
        .0
        .get_rule_progress()
        .into_iter()
        .map(|p| ProgressDto {
            rule_id: p.rule_id,
            step_index: p.step_index,
            step_count: p.step_count,
            completed_cycles: p.completed_cycles,
            paused: p.paused,
            error: p.error,
            lateness_ms: p.lateness_ms,
            max_lateness_ms: p.max_lateness_ms,
        })
        .collect();
    states
}

/// 面板聚焦时 WH_KEYBOARD_LL 不触发，前端将键盘事件中继到引擎统一处理。
/// 注意：WebView 默认行为必须由前端在 DOM 事件内同步 preventDefault；
/// 这里的 handled 只表示引擎是否处理了按键，不能用于事后取消 F3 等浏览器快捷键。
#[tauri::command]
pub fn relay_key_event(state: State<EngineState>, key: KeyId, is_up: bool) -> RelayKeyResult {
    if try_consume_relay_injection(key, is_up) {
        return RelayKeyResult {
            accepted_physical: false,
            handled: false,
        };
    }

    let result = if is_up {
        state.0.on_key_release_event(key)
    } else {
        state.0.on_key_press_event(key)
    };
    RelayKeyResult {
        accepted_physical: result.accepted_physical,
        handled: result.handled,
    }
}

#[tauri::command]
pub fn get_input_mode() -> String {
    #[cfg(windows)]
    {
        win_input::current_mode().as_str().to_string()
    }
    #[cfg(not(windows))]
    {
        "sendinput".to_string()
    }
}

#[tauri::command]
pub fn set_input_mode(
    app: AppHandle,
    state: State<EngineState>,
    mode: String,
    persist: Option<bool>,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        use win_input::InputMode;

        let input_mode =
            InputMode::from_str(&mode).ok_or_else(|| format!("未知输入模式: {mode}"))?;

        if input_mode.requires_admin() && !win_driver::elevation::is_process_elevated() {
            return Err(format!(
                "{} 需要管理员权限，请先以管理员身份重启应用",
                input_mode_label(input_mode)
            ));
        }

        if input_mode == InputMode::Interception {
            let health = crate::commands::status::collect_interception_health();
            if let Some(issue) = health.issue {
                tracing::warn!("游戏模式启用被阻止：{}", health.detail);
                switch_input_backend_runtime(&state.0, InputMode::SendInput)?;
                crate::commands::status::emit_status_changed(&app);
                return Err(format!("{issue} 已回退到通用模式，游戏模式未启用。"));
            }
        }

        check_rules_for_mode(&state.0.get_rules(), input_mode)?;

        if persist.unwrap_or(true) {
            switch_input_backend(&app, &state.0, input_mode)?;
        } else {
            switch_input_backend_runtime(&state.0, input_mode)?;
        }
        if win_input::current_mode() == InputMode::HidStick {
            crate::bootstrap::input::on_hid_stick_backend_up(&app);
        }

        crate::commands::status::emit_status_changed(&app);
        if win_input::current_mode() != input_mode {
            return Err(match input_mode {
                InputMode::DdSimple => "DD 驱动通道未就绪，已回退通用模式。请退出其他使用 DD 的程序后重试；如仍失败，请正常退出本应用后重开，并在诊断修复中检查驱动残留。".into(),
                _ => "输入通道初始化失败，已回退通用模式；请刷新诊断并检查设备连接或重启 Windows。".into(),
            });
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = (app, state, mode, persist);
        Err("仅 Windows 平台支持切换输入模式".to_string())
    }
}

/// 安全切换输入后端并持久化所选模式。切换期间通过 [`BurstEngine::begin_backend_switch`] 置
/// 「切换中」标志、停连发并经旧后端阻塞释放所有已按下的目标键，使窗口内到来的物理触发不会启动
/// 规则——杜绝目标键 down 走旧后端、up 走新后端的错配卡键。不改全局开关，用户在切换期间按下的
/// 停止热键得以保留。`set_input_mode` 与驱动卸载 / 修复共用此入口，避免各处切后端逻辑漂移。
#[cfg(windows)]
pub(crate) fn switch_input_backend(
    app: &AppHandle,
    engine: &BurstEngine,
    mode: win_input::InputMode,
) -> Result<(), String> {
    use tauri_plugin_store::StoreExt;
    switch_input_backend_runtime(engine, mode)?;
    if let Ok(store) = app.store(crate::STORE_PATH) {
        store.set("input_mode", serde_json::json!(mode.as_str()));
        let _ = store.save();
    }
    Ok(())
}

#[cfg(windows)]
fn switch_input_backend_runtime(
    engine: &BurstEngine,
    mode: win_input::InputMode,
) -> Result<(), String> {
    // 临时降级也必须先停止注入和抬键，但不能改写用户保存的模式。
    engine.begin_backend_switch()?;
    win_input::init_backend(mode);
    engine.end_backend_switch();
    Ok(())
}

/// 卸载 / 修复驱动前让出驱动通道：切回 SendInput。外设模式不经任何驱动，保持不动——否则用户为
/// 腾 Interception 设备槽去卸游戏模式驱动，会被顺手踢出外设模式，连保存的模式也改掉。
#[cfg(windows)]
pub(crate) fn leave_driver_backend(app: &AppHandle, engine: &BurstEngine) -> Result<(), String> {
    if win_input::current_mode() == win_input::InputMode::HidStick {
        return Ok(());
    }
    switch_input_backend(app, engine, win_input::InputMode::SendInput)
}

#[cfg(windows)]
fn input_mode_label(mode: win_input::InputMode) -> &'static str {
    match mode {
        win_input::InputMode::SendInput => "通用模式",
        win_input::InputMode::Interception => "游戏模式",
        win_input::InputMode::DdSimple => "DD驱动",
        win_input::InputMode::HidStick => "外设模式",
    }
}

/// 校验规则集是否满足目标输入模式的约束。
///
/// 判定本身全部委托 [`qzh_profile::find_rule_violations`]，与 `Profile::validate_for_mode`
/// 和加载期净化共用同一份表；这里只负责把机器可读的原因翻成用户看得懂的话。
/// `set_rules`（编辑规则保存）与 `set_input_mode`（切换模式）共用此入口。
#[cfg(windows)]
fn check_rules_for_mode(rules: &[BurstRule], mode: win_input::InputMode) -> Result<(), String> {
    let label = input_mode_label(mode);
    if let Some((rule, reason)) = find_rule_violations(rules, caps_for_mode(mode))
        .into_iter()
        .next()
    {
        return Err(format!(
            "规则「{rule}」{}",
            rejection_message(reason, label)
        ));
    }
    Ok(())
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use qzh_profile::{BurstMode, MouseButton};
    use win_input::InputMode;

    #[test]
    fn dd_accepts_coincident_toggle_in_capture_and_rule_validation() {
        for target in [KeyId::Keyboard(0x51), KeyId::Mouse(MouseButton::Left)] {
            let policy = slot_policy(
                KeySlot::TriggerTargetToggle,
                caps_for_mode(InputMode::DdSimple),
            );
            match target {
                KeyId::Keyboard(_) => assert!(policy.keyboard),
                KeyId::Mouse(button) => assert!(policy.mouse.contains(&button)),
            }
            for (trigger, stop) in [
                (target, None),
                (KeyId::Keyboard(0x46), Some(target)),
                (target, Some(target)),
            ] {
                let rule = BurstRule {
                    id: "same-key".into(),
                    enabled: true,
                    trigger_key: trigger,
                    steps: vec![qzh_profile::profile::OutputStep {
                        key: target,
                        interval_ms: 10,
                    }],
                    mode: BurstMode::Toggle,
                    stop_key: stop,
                    group: None,
                };
                assert!(
                    check_rules_for_mode(std::slice::from_ref(&rule), InputMode::DdSimple).is_ok()
                );
                assert!(check_rules_for_mode(&[rule], InputMode::HidStick).is_err());
            }
        }
    }
}
