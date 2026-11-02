//! 按键角色与录入策略的唯一事实来源。
//!
//! 系统里的按键只有两种角色：**读**角色只被低级钩子观察，永不注入；**写**角色只经
//! [`win_input::dispatch`] 注入，不参与触发判定。驱动支持只对写角色有意义。
//!
//! 一个槽位可能同时承担两种角色（默认模式与横版单键的连发按键，`trigger == target`），
//! 此时它受两套约束的**交集**。方向与权限模型的 `rw` 相反：权限枚举「被授予的动作」，
//! 角色越多动作越多故取并集；这里枚举「够格的按键」，角色越多要求越多故取交集。
//!
//! 限制来自两个不同的源，本模块把它们分开表达：
//! - **能力**：后端做不做得到。随输入模式变，由调用方通过 [`InjectCaps`] 传入
//!   （`win-input` 的 `InputMode` 谓词负责填充，本 crate 不依赖它以保持平台无关）。
//! - **策略**：产品上让不让做。与输入模式无关，写死在本模块。
//!
//! 前端通过 `get_key_policy` 命令读取本表，不再自行维护一份白名单。配置文件导入、
//! 托盘切换配置、启动加载等入口都不经过前端，故校验必须落在这里。

use serde::{Deserialize, Serialize};

use crate::key_id::{KeyId, MouseButton};
use crate::profile::{BurstMode, BurstRule, Hotkeys, Profile};

/// 左右分离的修饰键 VK（Shift / Ctrl / Alt / Win）。
///
/// 只允许绑全局热键：热键纯读，按下不产生任何注入；而连发 Alt / Win 会持续触发系统
/// 菜单语义，`WM_SYSKEY*` 通道下各游戏表现也不一致。这是产品策略不是能力限制——
/// 低级钩子看得见它们，`SendInput` 也注入得出去（扫描码走 `KEYEVENTF_EXTENDEDKEY`）。
pub const MODIFIER_VKS: [u32; 8] = [0xa0, 0xa1, 0xa2, 0xa3, 0xa4, 0xa5, 0x5b, 0x5c];

/// 全部鼠标按钮，用于枚举槽位允许集。新增 [`MouseButton`] 变体时必须同步补进来。
pub const ALL_MOUSE_BUTTONS: [MouseButton; 7] = [
    MouseButton::Left,
    MouseButton::Right,
    MouseButton::Middle,
    MouseButton::X1,
    MouseButton::X2,
    MouseButton::WheelUp,
    MouseButton::WheelDown,
];

/// 是否是左右修饰键。
pub fn is_modifier_vk(vk: u32) -> bool {
    MODIFIER_VKS.contains(&vk)
}

/// 一个按键录入位置。决定该处接受哪些键，也决定这个键承担什么角色。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum KeySlot {
    /// 三个全局热键（开启 / 停止 / 面板显隐）。纯读。
    Hotkey,
    /// 规则的启动键或停止键，且与连发按键不同。纯读。
    Trigger,
    /// 规则的连发按键，且与启动 / 停止键不同。纯写。
    Target,
    /// 启动键与连发按键重合，规则是长按连发。读写皆是，取两者交集。
    TriggerTarget,
    /// 启动键与连发按键重合，规则是切换连发。重合态对自注入过滤的要求比长按更高，
    /// 故与 [`Self::TriggerTarget`] 分开：后端不支持时这个槽位是空集，键盘键也不收。
    TriggerTargetToggle,
}

impl KeySlot {
    /// 该槽位的键是否会被注入（写角色）。决定它受不受后端能力约束。
    pub fn injects(self) -> bool {
        matches!(
            self,
            Self::Target | Self::TriggerTarget | Self::TriggerTargetToggle
        )
    }
}

/// 当前输入后端的注入能力。由 `win-input` 的 `InputMode` 谓词填充。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InjectCaps {
    /// 当前输入模式是否允许 Toggle 规则进入重合态，不代表自注入过滤可靠性。
    ///
    /// DD 系列把 `ExtraInformation` 写死为 0，`SIM_MARKER` 无法幸存，自注入只能靠时间
    /// 窗口队列过滤；该队列对重合键无法可靠区分「用户真实按下」与「自身回灌」，会导致
    /// 连发自停或停不掉。DD 接受此不可靠性，此项为 true；外设模式仍为 false。
    /// Hold 的重合态仍然放行——那份不可靠性是已知且已接受的，见 `win-input` 顶部注释。
    pub coincident_toggle: bool,
}

impl Default for InjectCaps {
    /// 无后端信息时按最宽松处理，交由运行时回退兜底。
    fn default() -> Self {
        Self {
            coincident_toggle: true,
        }
    }
}

/// 某个键在某个槽位被拒绝的原因。文案由调用方决定，本枚举只表达「为什么不行」。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum KeyRejection {
    /// 修饰键只能绑全局热键。
    ModifierNotAllowed,
    /// 该槽位不接受鼠标按键与滚轮（当前仅全局热键如此）。
    MouseNotAllowed,
    /// 当前输入模式不支持 Toggle 规则的启动 / 停止键与连发按键相同。
    CoincidentToggleUnsupported,
}

/// 判定某个键能否录入某个槽位。这是本模块唯一的判定入口，其余导出都由它派生。
pub fn accepts(slot: KeySlot, key: KeyId, caps: InjectCaps) -> Result<(), KeyRejection> {
    // 能力先判：后端撑不住这个槽位时，是哪个键都不重要。
    if slot == KeySlot::TriggerTargetToggle && !caps.coincident_toggle {
        return Err(KeyRejection::CoincidentToggleUnsupported);
    }
    match key {
        KeyId::Keyboard(vk) => {
            if is_modifier_vk(vk) && slot != KeySlot::Hotkey {
                return Err(KeyRejection::ModifierNotAllowed);
            }
            Ok(())
        }
        KeyId::Mouse(btn) => {
            if slot == KeySlot::Hotkey {
                return Err(KeyRejection::MouseNotAllowed);
            }
            // 现存后端（SendInput / Interception / DDSimple）都注入得了全部鼠标按钮与滚轮。
            // 唯一有值域缺口的是已停用的 DD-HID，随其一并移除；将来若有后端按钮值域受限，
            // 在 InjectCaps 加能力位并在此判定即可，SlotPolicy 会自动跟着变。
            let _ = btn;
            Ok(())
        }
    }
}

/// 一个槽位的允许集，供前端按键捕获组件直接查表。
///
/// 键盘普通键一律允许，故不单列；前端自己的 `code` → VK 映射决定它认得哪些键。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SlotPolicy {
    /// 是否接受普通键盘键。为 false 时该槽位在当前后端下不可用，任何键都收不了。
    pub keyboard: bool,
    /// 是否接受左右修饰键。
    pub modifiers: bool,
    /// 接受的鼠标按钮；空数组表示该槽位完全不接受鼠标与滚轮。
    pub mouse: Vec<MouseButton>,
}

/// 用于探测「普通键盘键收不收」的样本 VK。取字母 A，不是修饰键也不是任何特殊键。
const SAMPLE_PLAIN_VK: u32 = 0x41;

/// 导出某个槽位的允许集。逐个键走 [`accepts`] 过滤，保证描述与判定不可能漂移。
pub fn slot_policy(slot: KeySlot, caps: InjectCaps) -> SlotPolicy {
    SlotPolicy {
        keyboard: accepts(slot, KeyId::Keyboard(SAMPLE_PLAIN_VK), caps).is_ok(),
        modifiers: accepts(slot, KeyId::Keyboard(MODIFIER_VKS[0]), caps).is_ok(),
        mouse: ALL_MOUSE_BUTTONS
            .into_iter()
            .filter(|btn| accepts(slot, KeyId::Mouse(*btn), caps).is_ok())
            .collect(),
    }
}

/// 把不符合策略的全局热键置空，返回被清掉的槽位名供调用方记日志。
///
/// 用于配置文件加载路径：`.qzh` 可能来自他人分享或手工编辑，绕过了前端的录入拦截。
/// 只清违规项而不拒绝整个文件——拒绝会让用户彻底打不开自己的配置。
/// 这里只检查与输入模式无关的结构策略，模式相关的约束由 `check_rules_for_mode` 负责。
pub fn sanitize_hotkeys(hotkeys: &mut Hotkeys) -> Vec<&'static str> {
    let caps = InjectCaps::default();
    let mut dropped = Vec::new();
    let mut check = |slot: &mut Option<KeyId>, name: &'static str| {
        if let Some(key) = *slot {
            if accepts(KeySlot::Hotkey, key, caps).is_err() {
                *slot = None;
                dropped.push(name);
            }
        }
    };
    check(&mut hotkeys.global_toggle, "global_toggle");
    check(&mut hotkeys.global_stop, "global_stop");
    check(&mut hotkeys.panel_toggle, "panel_toggle");
    dropped
}

/// 判定某个键在一条规则里落在哪个槽位。
///
/// 这是「角色 × 是否重合」模型的可执行形式：既被读（启动键或停止键）又被写（连发按键）
/// 的键落在 [`KeySlot::TriggerTarget`]，受两套约束的交集。
pub fn slot_of(rule: &BurstRule, key: KeyId) -> KeySlot {
    let stop = rule.stop_key.unwrap_or(rule.trigger_key);
    let reads = key == rule.trigger_key || key == stop;
    let writes = rule.steps.iter().any(|step| step.key == key);
    match (reads, writes) {
        (true, true) if rule.mode == BurstMode::Toggle => KeySlot::TriggerTargetToggle,
        (true, true) => KeySlot::TriggerTarget,
        (false, true) => KeySlot::Target,
        _ => KeySlot::Trigger,
    }
}

/// 找出规则集里不符合策略的按键，返回 `(规则 id, 原因)` 供调用方记日志或拒绝保存。
///
/// 与 [`sanitize_hotkeys`] 的处置方式刻意不同：违规热键会被清空，因为一个绑到鼠标左键的
/// 全局开关会让每次点击都切换连发、应用直接不可用；违规规则只被报出来不被改写，因为影响
/// 范围止于该条规则，用户在界面上看得见也改得掉，静默禁用反而更像 bug。
pub fn find_rule_violations(rules: &[BurstRule], caps: InjectCaps) -> Vec<(String, KeyRejection)> {
    let mut out = Vec::new();
    for rule in rules.iter().filter(|r| r.enabled) {
        let stop = rule.stop_key.unwrap_or(rule.trigger_key);
        let mut seen: Vec<KeyId> = Vec::new();
        for key in [rule.trigger_key, stop]
            .into_iter()
            .chain(rule.steps.iter().map(|step| step.key))
        {
            if seen.contains(&key) {
                continue;
            }
            seen.push(key);
            if let Err(reason) = accepts(slot_of(rule, key), key, caps) {
                out.push((rule.id.clone(), reason));
            }
        }
    }
    out
}

/// 一条被停用的规则及其原因。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DisabledRule {
    /// 规则 id。
    pub id: String,
    /// 停用原因。
    pub reason: KeyRejection,
}

/// 加载期净化的结果，供调用方记日志并提醒用户。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct SanitizeReport {
    /// 被置为未绑定的全局热键槽位名。
    pub cleared_hotkeys: Vec<&'static str>,
    /// 被停用的规则。
    pub disabled_rules: Vec<DisabledRule>,
}

impl SanitizeReport {
    /// 没有任何改动。
    pub fn is_empty(&self) -> bool {
        self.cleared_hotkeys.is_empty() && self.disabled_rules.is_empty()
    }
}

/// 加载期净化：把不符合策略的配置改成安全状态，返回改动报告。
///
/// 两种处置方式刻意不同：
/// - **热键置为未绑定**。未绑定本来就是热键的合法状态（`Option<KeyId>` 的 `None`），
///   改成它不产生任何新状态。而一个绑到鼠标左键的全局开关会让每次点击都切换连发，
///   应用直接不可用，必须拆掉。
/// - **规则只停用，不动按键**。把规则里的键改写成「空」会造出一个此前不存在的中间态，
///   连发索引、冲突检测、界面渲染都得为它加分支，连锁面太大。停用既止住了行为，
///   又完整保留用户配置，用户在界面上看得见、改完重新勾选即可。
///
/// 只处理与输入模式无关的结构策略时传 [`InjectCaps::default`]；要连当前后端的注入能力
/// 一起收紧，传实际能力。
pub fn sanitize_profile(profile: &mut Profile, caps: InjectCaps) -> SanitizeReport {
    let disabled: Vec<DisabledRule> = find_rule_violations(&profile.rules, caps)
        .into_iter()
        .map(|(id, reason)| DisabledRule { id, reason })
        .collect();
    for entry in &disabled {
        if let Some(rule) = profile.rules.iter_mut().find(|r| r.id == entry.id) {
            rule.enabled = false;
        }
    }
    SanitizeReport {
        cleared_hotkeys: sanitize_hotkeys(&mut profile.hotkeys),
        disabled_rules: disabled,
    }
}

#[cfg(test)]
#[path = "key_policy_tests.rs"]
mod tests;
