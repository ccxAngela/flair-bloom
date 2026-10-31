mod rules;
mod scheduler;
pub mod stress;

#[cfg(test)]
mod pipeline_tests;
#[cfg(test)]
mod preempt_tests;
#[cfg(test)]
mod test_support;

#[cfg(all(test, windows))]
mod smoke_tests;

use qzh_profile::key_id::KeyId;
#[cfg(any(test, windows))]
use qzh_profile::key_id::MouseButton;
use qzh_profile::profile::{BurstMode, BurstRule, Hotkeys};
use rules::RuleSnapshot;
use scheduler::{release_simulated_keys, PhysicalKeys, Scheduler, SchedulerHandle, SimulatedKeys};
#[cfg(windows)]
use std::sync::{atomic::AtomicU32, RwLock, Weak};
#[cfg(windows)]
use std::thread;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        Arc, Mutex,
    },
};
#[cfg(windows)]
use tracing::error;
use tracing::info;
use win_input::{clear_pending_injections, clear_relay_injections};
#[cfg(windows)]
use win_input::{try_consume_injection, SIM_MARKER};
#[cfg(windows)]
use windows_sys::Win32::{
    Foundation::{LPARAM, WPARAM},
    System::Threading::GetCurrentThreadId,
    UI::WindowsAndMessaging::{
        CallNextHookEx, DispatchMessageW, GetMessageW, PostThreadMessageW, SetWindowsHookExW,
        TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT, MSG, MSLLHOOKSTRUCT,
        WH_KEYBOARD_LL, WH_MOUSE_LL, WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP,
        WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
        WM_SYSKEYUP, WM_USER, WM_XBUTTONDOWN, WM_XBUTTONUP, XBUTTON1, XBUTTON2,
    },
};

/// 重装键盘 hook 的自定义线程消息：面板获得焦点时，由主线程投递给 hook 线程，
/// 触发 unhook + rehook 使我们的 hook 重新排到 Chromium hook 之后安装，即优先被调用。
#[cfg(windows)]
const WM_REHOOK_KEYBOARD: u32 = WM_USER + 1;

/// hook 线程 ID，用于跨线程投递 WM_REHOOK_KEYBOARD 消息；0 表示线程尚未启动。
#[cfg(windows)]
static HOOK_THREAD_ID: AtomicU32 = AtomicU32::new(0);

/// hook 回调通过静态 Weak 引用访问引擎，避免 Arc 延长生命周期；RwLock 支持重复注册
#[cfg(windows)]
static ENGINE_HOOK: RwLock<Option<Weak<BurstEngine>>> = RwLock::new(None);

/// 向 hook 线程投递重装键盘 hook 的信号。
/// 在面板窗口获得焦点后调用，使我们的 hook 重新排到 Chromium 安装的 hook 之后（即优先执行）。
#[cfg(windows)]
pub fn rehook_keyboard() {
    let tid = HOOK_THREAD_ID.load(Ordering::SeqCst);
    if tid != 0 {
        // SAFETY: tid 来自 hook 线程自身写入的有效线程 ID；消息参数均为 0，合法。
        unsafe { PostThreadMessageW(tid, WM_REHOOK_KEYBOARD, 0, 0) };
    }
}
#[cfg(not(windows))]
pub fn rehook_keyboard() {}

type GlobalChangedCb = Arc<Mutex<Option<Box<dyn Fn(bool) + Send + Sync>>>>;
type PanelToggleCb = Arc<Mutex<Option<Box<dyn Fn() + Send + Sync>>>>;

/// 关键路径锁兜底：若前一持锁者 panic 导致 Mutex 中毒,强行复活并继续。
/// 按键工具最差故障是键盘卡死,值得给一层硬兜底；连发线程已被 catch_unwind
/// 包裹,持锁期间也仅做 HashMap/Vec 操作,正常路径不会中毒。
fn revive<T>(r: std::sync::LockResult<T>) -> T {
    r.unwrap_or_else(|e| e.into_inner())
}

/// 从 hook 静态弱引用取出存活的引擎，**容忍 RwLock 中毒**。与 [`revive`] 同一安全立场：
/// 按键工具最差故障是卡键，绝不能因锁中毒在 `extern "system"` 回调里 panic——跨 FFI 边界
/// 展开是 abort/UB。读锁只在内部短暂持有，中毒后取 `into_inner` 继续是安全的。
#[cfg(windows)]
fn engine_from_hook() -> Option<Arc<BurstEngine>> {
    ENGINE_HOOK
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .and_then(|w| w.upgrade())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EngineLifecycle {
    Paused,
    Running,
    Stopping,
    ShuttingDown,
    Shutdown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyProcessResult {
    pub accepted_physical: bool,
    pub handled: bool,
}

/// 规则的运行 / 暂停分区。`running` 正在向调度器连发；`paused` 仍算开启（按着的长按、
/// 已开启的切换），只是被同组按住的长按插队而暂时让位。两列均按 ID 排序。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuleStates {
    pub running: Vec<String>,
    pub paused: Vec<String>,
}

#[derive(Debug, Clone)]
struct ActiveRule {
    rule: Arc<BurstRule>,
    /// 被同组长按插队而让位：仍算开启，但调度器里已停；恢复时重新 `start_rule`。
    paused: bool,
}

impl ActiveRule {
    fn in_group(&self, group: &str) -> bool {
        self.rule.group.as_deref() == Some(group)
    }
}

#[derive(Debug)]
struct RuntimeState {
    lifecycle: EngineLifecycle,
    /// 当前活跃规则（含被插队暂停的）。Toggle 是否「已开启」直接由此派生，
    /// 以单一事实来源避免双表同步竞态。
    active_rules: HashMap<String, ActiveRule>,
    /// 组内插队栈：group → 正按住的组内长按规则 ID，按按下顺序，栈顶在跑。按组分桶使跨组
    /// 长按天然互不影响。不变量：某组栈非空时，该组只有栈顶在跑，其余同组活跃规则都 paused；
    /// 栈空即删键，使「组里有没有长按按着」只需查键是否存在，不会被残留的空栈误判。
    hold_stacks: HashMap<String, Vec<String>>,
    stop_generation: u64,
}

impl RuntimeState {
    /// 清空全部活跃规则与插队栈（全局暂停、换规则、切后端、退出共用）。
    fn clear_activity(&mut self) {
        self.active_rules.clear();
        self.hold_stacks.clear();
    }
}

pub struct BurstEngine {
    pub global_enabled: Arc<AtomicBool>,
    rules: Arc<Mutex<RuleSnapshot>>,
    runtime: Arc<Mutex<RuntimeState>>,
    physical_pressed: PhysicalKeys,
    simulated_keys: SimulatedKeys,
    scheduler: Arc<dyn Scheduler>,
    hotkeys: Arc<Mutex<Hotkeys>>,
    /// 专用全局停止键（`global_stop` 已设且与 `global_toggle` 不同）的无锁缓存，
    /// `0` 表示未设置。物理输入热路径据此免锁判定停止键，保证紧急停止 100% 触发
    /// 又不在每次按键时加 `hotkeys` 锁。由 [`set_hotkeys`] 维护。
    dedicated_stop: AtomicU64,
    /// 后端切换进行中标志。置位期间物理触发不启动任何规则，使「停旧后端连发 → init_backend
    /// 换后端」窗口内不会出现目标键 down 走旧后端、up 走新后端的错配卡键。不动 `global_enabled`，
    /// 故用户在切换窗口内按下的全局停止热键得以保留。由 [`BurstEngine::begin_backend_switch`] /
    /// [`BurstEngine::end_backend_switch`] 维护。
    switching_backend: AtomicBool,
    /// 全局开关状态被热键改变时调用（由 app 层注册，用于同步托盘与前端）。
    on_global_changed: GlobalChangedCb,
    /// 面板显隐热键触发时调用。
    on_panel_toggle: PanelToggleCb,
}

/// 把 [`KeyId`] 编码为非零 `u64`，供 `dedicated_stop` 无锁缓存比较。`0` 保留给「未设置」。
fn encode_key_id(key: KeyId) -> u64 {
    match key {
        KeyId::Keyboard(vk) => (1u64 << 40) | u64::from(vk),
        KeyId::Mouse(button) => (2u64 << 40) | (button as u64),
    }
}

impl Default for BurstEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl BurstEngine {
    pub fn new() -> Self {
        let simulated_keys = Arc::new(Mutex::new(HashMap::new()));
        let scheduler: Arc<dyn Scheduler> =
            Arc::new(SchedulerHandle::start(simulated_keys.clone()));
        Self::from_parts(scheduler, simulated_keys)
    }

    fn from_parts(scheduler: Arc<dyn Scheduler>, simulated_keys: SimulatedKeys) -> Self {
        Self {
            global_enabled: Arc::new(AtomicBool::new(false)),
            rules: Arc::new(Mutex::new(RuleSnapshot::default())),
            runtime: Arc::new(Mutex::new(RuntimeState {
                lifecycle: EngineLifecycle::Paused,
                active_rules: HashMap::new(),
                hold_stacks: HashMap::new(),
                stop_generation: 0,
            })),
            physical_pressed: Arc::new(Mutex::new(HashSet::new())),
            simulated_keys,
            scheduler,
            hotkeys: Arc::new(Mutex::new(Hotkeys::default())),
            dedicated_stop: AtomicU64::new(0),
            switching_backend: AtomicBool::new(false),
            on_global_changed: Arc::new(Mutex::new(None)),
            on_panel_toggle: Arc::new(Mutex::new(None)),
        }
    }

    /// 测试构造：注入自定义调度器（如命令录制替身），对引擎→调度命令做确定性断言。
    #[cfg(test)]
    pub(crate) fn new_with_scheduler(scheduler: Arc<dyn Scheduler>) -> Self {
        Self::from_parts(scheduler, Arc::new(Mutex::new(HashMap::new())))
    }

    pub fn set_hotkeys(&self, hotkeys: Hotkeys) {
        // 缓存专用停止键（已设 global_stop 且与 global_toggle 不同）供热路径免锁判定。
        let dedicated = match hotkeys.global_stop {
            Some(stop) if hotkeys.global_toggle != Some(stop) => encode_key_id(stop),
            _ => 0,
        };
        self.dedicated_stop.store(dedicated, Ordering::Relaxed);
        *revive(self.hotkeys.lock()) = hotkeys;
    }

    /// 兼容旧调用名：停止所有活动规则，并等待 scheduler 完成目标键释放。
    pub fn cancel_all_loops(&self) {
        self.stop_runtime_activity(true);
    }

    /// 进入后端切换：置「切换中」标志并停连发、经当前（旧）后端阻塞释放所有已按下的目标键。
    /// 标志置位后物理触发不再启动规则，故切换窗口内不会产生 down/up 跨后端错配。必须与
    /// [`BurstEngine::end_backend_switch`] 成对调用（中间执行 `init_backend`）。
    pub fn begin_backend_switch(&self) {
        self.switching_backend.store(true, Ordering::SeqCst);
        self.stop_runtime_activity(true);
    }

    /// 结束后端切换：清除「切换中」标志，恢复物理触发→规则启动。全局开关状态未被本过程改动，
    /// 故按其真实值（含切换期间用户按下的停止）自然恢复。
    pub fn end_backend_switch(&self) {
        self.switching_backend.store(false, Ordering::SeqCst);
    }

    /// 注册全局开关热键触发时的回调（供 app 层同步托盘与前端事件）。
    pub fn set_on_global_changed(&self, f: impl Fn(bool) + Send + Sync + 'static) {
        *revive(self.on_global_changed.lock()) = Some(Box::new(f));
    }

    /// 注册面板显隐热键触发时的回调。
    pub fn set_on_panel_toggle(&self, f: impl Fn() + Send + Sync + 'static) {
        *revive(self.on_panel_toggle.lock()) = Some(Box::new(f));
    }

    pub fn set_rules(&self, rules: Vec<BurstRule>) {
        // 先换快照、再停活动。停止会自增 generation 并向调度器发 StopAll，扫掉任何「在换快照前用
        // 旧快照已经启动」的规则；若顺序反过来（先停后换），换快照后到来的物理触发会用旧快照在新
        // generation 上启动一条已删除的规则，而其释放查的是新快照（找不到）→ 永不停止、持续连发。
        *revive(self.rules.lock()) = RuleSnapshot::new(rules);
        self.stop_runtime_activity(true);
    }

    pub fn get_rules(&self) -> Vec<BurstRule> {
        revive(self.rules.lock()).rules()
    }

    /// 当前生效的全局热键。前端在写入被拒时用它回滚，避免界面与引擎分叉。
    pub fn get_hotkeys(&self) -> Hotkeys {
        revive(self.hotkeys.lock()).clone()
    }

    /// 当前活跃的规则 ID 集合（含被插队暂停的）：hold 模式表示触发键被按住，toggle 模式表示已开启。
    /// 前端靠它差分播报切换规则启停，暂停 / 恢复不改变该集合，故插队不会多响。
    pub fn get_active_ids(&self) -> Vec<String> {
        let mut ids = revive(self.runtime.lock())
            .active_rules
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        ids.sort();
        ids
    }

    /// 活跃规则按运行 / 暂停分区，供前端区分「在跑」与「被插队暂停」。
    pub fn get_rule_states(&self) -> RuleStates {
        let mut states = RuleStates::default();
        for (id, active) in &revive(self.runtime.lock()).active_rules {
            if active.paused {
                states.paused.push(id.clone());
            } else {
                states.running.push(id.clone());
            }
        }
        states.running.sort();
        states.paused.sort();
        states
    }

    /// 返回 true 表示引擎处理了本次按键（热键触发或规则匹配），false 表示未匹配或重复按下。
    /// 供中继调用方决定是否 preventDefault。
    pub fn on_key_press(&self, key: KeyId) -> bool {
        self.on_key_press_event(key).handled
    }

    pub fn on_key_release(&self, key: KeyId) {
        let _ = self.on_key_release_event(key);
    }

    pub fn on_key_press_event(&self, key: KeyId) -> KeyProcessResult {
        let is_new_physical = revive(self.physical_pressed.lock()).insert(key);

        // 安全底线：专用全局停止键 100% 可触发。用无锁原子缓存判定，物理输入热路径不新增
        // 跨线程锁；先于去重——即便 physical_pressed 因注入回灌残留幻影项导致
        // is_new_physical=false，停止仍会触发。停止幂等且受 `enabled` 拦截重复，专用停止键
        // 不是 start 键，无 enable/disable 抖动。
        let stop_cache = self.dedicated_stop.load(Ordering::Relaxed);
        if stop_cache != 0
            && stop_cache == encode_key_id(key)
            && self.global_enabled.load(Ordering::SeqCst)
        {
            self.set_global_enabled_and_notify(false);
            return KeyProcessResult {
                accepted_physical: is_new_physical,
                handled: true,
            };
        }

        if !is_new_physical {
            return KeyProcessResult {
                accepted_physical: false,
                handled: false,
            };
        }

        if self.handle_hotkey_press(key) {
            return KeyProcessResult {
                accepted_physical: true,
                handled: true,
            };
        }

        // 后端切换中或全局未启用：放行物理事件但不启动任何规则。切换中拦截规则启动，避免目标键
        // down/up 跨新旧后端错配卡键；停止键与热键已在上方先行处理，不受此拦截影响。
        if self.switching_backend.load(Ordering::SeqCst)
            || !self.global_enabled.load(Ordering::SeqCst)
        {
            return KeyProcessResult {
                accepted_physical: true,
                handled: false,
            };
        }

        let rules = revive(self.rules.lock()).enabled_press_rules(key);
        let mut handled = false;
        for rule in rules {
            handled = self.handle_rule_press(key, rule) || handled;
        }
        KeyProcessResult {
            accepted_physical: true,
            handled,
        }
    }

    pub fn on_key_release_event(&self, key: KeyId) -> KeyProcessResult {
        let accepted_physical = revive(self.physical_pressed.lock()).remove(&key);
        if !accepted_physical {
            return KeyProcessResult {
                accepted_physical: false,
                handled: false,
            };
        }

        let rules = revive(self.rules.lock()).enabled_hold_release_rules(key);
        let mut handled = false;
        for rule in rules {
            handled = self.stop_rule(&rule.id) || handled;
        }
        KeyProcessResult {
            accepted_physical,
            handled,
        }
    }

    pub fn set_global_enabled(&self, enabled: bool, wait: bool) {
        if enabled {
            // 在同一把 runtime 锁内判定关停态并落定 global_enabled + lifecycle，避免两类割裂：
            // (1) 关停期间被热键开启复活成 Running、且 global_enabled 残留 true（调度器已拆，状态割裂）；
            // (2) global_enabled 先 true、lifecycle 后 Running 的窗口里物理触发被 runtime_can_start 误丢。
            let mut runtime = revive(self.runtime.lock());
            if matches!(
                runtime.lifecycle,
                EngineLifecycle::ShuttingDown | EngineLifecycle::Shutdown
            ) {
                return;
            }
            self.global_enabled.store(true, Ordering::SeqCst);
            runtime.lifecycle = EngineLifecycle::Running;
        } else {
            // 与 enable 分支对称：在 runtime 锁内落定 global_enabled=false，使开关状态与后续的
            // lifecycle 判定串行化（lifecycle 最终由 stop_runtime_activity 末尾按 global_enabled
            // 收敛为 Paused）。阻塞式停止刻意放在锁外执行——低级钩子线程会读 global_enabled，
            // 绝不能在持锁期间跨线程阻塞导致系统输入卡顿。
            {
                let _runtime = revive(self.runtime.lock());
                self.global_enabled.store(false, Ordering::SeqCst);
            }
            self.stop_runtime_activity(wait);
        }
    }

    pub fn shutdown(&self) {
        let generation = {
            let mut runtime = revive(self.runtime.lock());
            if runtime.lifecycle == EngineLifecycle::Shutdown {
                return;
            }
            runtime.lifecycle = EngineLifecycle::ShuttingDown;
            runtime.stop_generation = runtime.stop_generation.saturating_add(1);
            runtime.clear_activity();
            runtime.stop_generation
        };
        self.global_enabled.store(false, Ordering::SeqCst);
        if !self.scheduler.shutdown_blocking(generation) {
            release_simulated_keys(&self.simulated_keys);
        }
        clear_pending_injections();
        clear_relay_injections();
        self.reset_physical_ledger();
        #[cfg(windows)]
        win_input::release_backends();
        revive(self.runtime.lock()).lifecycle = EngineLifecycle::Shutdown;
    }

    pub fn scheduler_hp_degraded(&self) -> bool {
        self.scheduler.hp_degraded()
    }

    pub fn lifecycle(&self) -> EngineLifecycle {
        revive(self.runtime.lock()).lifecycle
    }

    /// 切换全局开关并通知 app 层（同步托盘与前端）。热键改变开关状态的统一出口。
    fn set_global_enabled_and_notify(&self, enabled: bool) {
        self.set_global_enabled(enabled, false);
        if let Some(cb) = revive(self.on_global_changed.lock()).as_ref() {
            cb(enabled);
        }
    }

    fn handle_hotkey_press(&self, key: KeyId) -> bool {
        let hk = revive(self.hotkeys.lock());
        let start = hk.global_toggle;
        let stop = hk.global_stop.or(start);
        let panel = hk.panel_toggle;
        let enabled = self.global_enabled.load(Ordering::SeqCst);
        drop(hk);

        if panel == Some(key) {
            if let Some(cb) = revive(self.on_panel_toggle.lock()).as_ref() {
                cb();
            }
            return true;
        }
        if start == Some(key) && !enabled {
            self.set_global_enabled_and_notify(true);
            return true;
        }
        if stop == Some(key) && enabled {
            self.set_global_enabled_and_notify(false);
            return true;
        }
        false
    }

    fn handle_rule_press(&self, key: KeyId, rule: Arc<BurstRule>) -> bool {
        match rule.mode {
            BurstMode::Hold => {
                if rule.trigger_key != key {
                    return false;
                }
                // 滚轮触发键无法「按住」：每格作为一次瞬发点按——委派调度器一次性点按，
                // 不进 active_rules、不靠 release 停止（详见 tap_once_rule 与 rules.rs 索引排除）。
                if key.is_wheel() {
                    self.tap_once_rule(rule)
                } else {
                    self.start_rule(rule)
                }
            }
            BurstMode::Toggle => self.handle_toggle_press(key, rule),
        }
    }

    fn handle_toggle_press(&self, key: KeyId, rule: Arc<BurstRule>) -> bool {
        let stop = rule.stop_key.unwrap_or(rule.trigger_key);
        let mut start_rule = false;
        let mut stop_ids = Vec::new();
        {
            let mut runtime = revive(self.runtime.lock());
            if !runtime_can_start(&runtime) {
                return false;
            }
            let started = runtime
                .active_rules
                .get(&rule.id)
                .is_some_and(|a| a.rule.mode == BurstMode::Toggle);
            if started {
                if stop != key {
                    return false;
                }
                // 被插队暂停的规则调度器里已停，只移出活跃集合，松手后也就不再恢复。
                if runtime
                    .active_rules
                    .remove(&rule.id)
                    .is_some_and(|a| !a.paused)
                {
                    stop_ids.push(rule.id.clone());
                }
            } else {
                if rule.trigger_key != key {
                    return false;
                }
                let mut paused = false;
                if let Some(group) = rule.group.as_deref() {
                    let displaced = runtime
                        .active_rules
                        .iter()
                        .filter_map(|(id, active)| {
                            (id != &rule.id
                                && active.rule.mode == BurstMode::Toggle
                                && active.in_group(group))
                            .then_some(id.clone())
                        })
                        .collect::<Vec<_>>();
                    for id in displaced {
                        if runtime.active_rules.remove(&id).is_some_and(|a| !a.paused) {
                            stop_ids.push(id);
                        }
                    }
                    // 同组有长按按着：正在按住的长按不被切换挤掉，新规则只成为松手后的恢复目标。
                    paused = runtime.hold_stacks.contains_key(group);
                }
                runtime.active_rules.insert(
                    rule.id.clone(),
                    ActiveRule {
                        rule: rule.clone(),
                        paused,
                    },
                );
                start_rule = !paused;
            }
            let generation = runtime.stop_generation;
            // 命令在锁内发出，见 [`BurstEngine::start_rule`]。
            for id in stop_ids {
                self.scheduler.stop_rule(id, generation);
            }
            if start_rule {
                self.scheduler.start_rule(rule, generation);
            }
        }
        true
    }

    /// 长按按下。带 group 时插队：同组正在跑的规则暂停让位，自身入栈开跑。
    ///
    /// 启停命令必须在 runtime 锁内发给调度器：面板聚焦时键盘走 relay 命令线程、鼠标走钩子线程，
    /// 两个线程可能同时改同一组。锁外发送时，状态变更的先后与命令到达调度器的先后可能相反——
    /// 比如松开长按要恢复切换 T、同时 T 的停止键把它移出活跃集，`stop T` 先到、`start T` 后到，
    /// 调度器里留下一条引擎以为已停的连发。发送只是无界通道入队加 SetEvent，不阻塞，
    /// 持锁发送不会拖慢钩子线程。
    fn start_rule(&self, rule: Arc<BurstRule>) -> bool {
        let mut stop_ids = Vec::new();
        {
            let mut runtime = revive(self.runtime.lock());
            if !runtime_can_start(&runtime) || runtime.active_rules.contains_key(&rule.id) {
                return false;
            }
            if let Some(group) = rule.group.as_deref() {
                for (id, active) in runtime.active_rules.iter_mut() {
                    if !active.paused && active.in_group(group) {
                        active.paused = true;
                        stop_ids.push(id.clone());
                    }
                }
                runtime
                    .hold_stacks
                    .entry(group.to_string())
                    .or_default()
                    .push(rule.id.clone());
            }
            runtime.active_rules.insert(
                rule.id.clone(),
                ActiveRule {
                    rule: rule.clone(),
                    paused: false,
                },
            );
            let generation = runtime.stop_generation;
            for id in stop_ids {
                self.scheduler.stop_rule(id, generation);
            }
            self.scheduler.start_rule(rule, generation);
        }
        true
    }

    /// 滚轮 Hold 的一次性点按：仅校验 lifecycle、捕获 generation 后委派调度器点按一次。
    /// 不登记 `active_rules`——瞬发且可重复，登记会被去重/释放逻辑误吞后续滚轮格。
    /// 也不参与组内插队：每格只是一次点按，没有「按住」可言，与同组切换同时跑即可。
    fn tap_once_rule(&self, rule: Arc<BurstRule>) -> bool {
        let generation = {
            let runtime = revive(self.runtime.lock());
            if !runtime_can_start(&runtime) {
                return false;
            }
            runtime.stop_generation
        };
        self.scheduler.tap_once(rule, generation);
        true
    }

    /// 长按松开。在跑的组内长按松手后把组还给上一个：栈里仍按着的前一条长按，
    /// 否则被暂停的切换规则；本就被暂停（栈中间先松）的只出栈，不影响栈顶。
    fn stop_rule(&self, rule_id: &str) -> bool {
        let mut resume = None;
        let mut runtime = revive(self.runtime.lock());
        let Some(active) = runtime.active_rules.remove(rule_id) else {
            return false;
        };
        if let Some(group) = active.rule.group.as_deref() {
            let top = runtime.hold_stacks.get_mut(group).and_then(|stack| {
                stack.retain(|id| id != rule_id);
                stack.last().cloned()
            });
            if top.is_none() {
                runtime.hold_stacks.remove(group);
            }
            if !active.paused {
                let candidate = top.or_else(|| {
                    runtime
                        .active_rules
                        .iter()
                        .find(|(_, a)| {
                            a.paused && a.rule.mode == BurstMode::Toggle && a.in_group(group)
                        })
                        .map(|(id, _)| id.clone())
                });
                if let Some(next) = candidate.and_then(|id| runtime.active_rules.get_mut(&id)) {
                    next.paused = false;
                    resume = Some(next.rule.clone());
                }
            }
        }
        let generation = runtime.stop_generation;
        // 与切换互斥替换同一契约：先停后启，调度器里同组任一时刻至多一条在跑。
        // 命令在锁内发出，见 [`BurstEngine::start_rule`]。
        if !active.paused {
            self.scheduler.stop_rule(rule_id.to_string(), generation);
        }
        if let Some(rule) = resume {
            self.scheduler.start_rule(rule, generation);
        }
        true
    }

    /// 复位 hook 维护的物理按键账本。仅在停止 / 暂停 / 切模式 / 退出等低频复位点调用，
    /// 清掉注入回灌偶发漏过过滤而残留的「幻影按下」，避免之后真实物理按键被
    /// `on_key_press_event` 的去重逻辑误吞。引擎维护自身不变量的安全底线，不依赖
    /// 调度器跨线程兜底（调度器绝不触碰该集合，物理输入热路径因此无跨线程锁争用）。
    fn reset_physical_ledger(&self) {
        revive(self.physical_pressed.lock()).clear();
    }

    fn stop_runtime_activity(&self, wait: bool) {
        let generation = {
            let mut runtime = revive(self.runtime.lock());
            if matches!(
                runtime.lifecycle,
                EngineLifecycle::ShuttingDown | EngineLifecycle::Shutdown
            ) {
                return;
            }
            runtime.lifecycle = EngineLifecycle::Stopping;
            runtime.stop_generation = runtime.stop_generation.saturating_add(1);
            runtime.clear_activity();
            runtime.stop_generation
        };

        if wait {
            if !self.scheduler.stop_all_blocking(generation) {
                release_simulated_keys(&self.simulated_keys);
            }
        } else {
            self.scheduler.stop_all_async(generation);
        }
        clear_pending_injections();
        clear_relay_injections();
        self.reset_physical_ledger();

        let mut runtime = revive(self.runtime.lock());
        if runtime.lifecycle != EngineLifecycle::Shutdown {
            runtime.lifecycle = if self.global_enabled.load(Ordering::SeqCst) {
                EngineLifecycle::Running
            } else {
                EngineLifecycle::Paused
            };
        }
    }
}

impl Drop for BurstEngine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn runtime_can_start(runtime: &RuntimeState) -> bool {
    runtime.lifecycle == EngineLifecycle::Running
}

/// WH_KEYBOARD_LL 低级键盘钩子回调；运行在安装 hook 的线程（消息循环线程）上。
///
/// # Safety
///
/// 由 Windows 调用,调用方契约：当 `ncode >= 0` 时 `lparam` 指向 Windows 维护的
/// 有效 `KBDLLHOOKSTRUCT`,生命周期覆盖本次回调返回前。函数内不持有该指针的延长引用,
/// 也不跨线程发送借用。
#[cfg(windows)]
unsafe extern "system" fn keyboard_hook_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> isize {
    if ncode >= 0 {
        // SAFETY: 上文 # Safety 契约保证 ncode>=0 时 lparam 是有效的
        // KBDLLHOOKSTRUCT 指针,借用 kb 不存活到回调返回之后
        let kb = &*(lparam as *const KBDLLHOOKSTRUCT);
        // SendInput / Interception：通过 dwExtraInfo 精确过滤自身注入；
        // DD-HID：dwExtraInfo 由驱动端置位，无法控制，转用 PENDING_INJECTIONS 队列匹配
        let is_sim_marker = kb.dwExtraInfo == SIM_MARKER;
        if !is_sim_marker {
            let key = KeyId::Keyboard(kb.vkCode);
            let is_up = matches!(wparam as u32, WM_KEYUP | WM_SYSKEYUP);
            let is_down_or_up = matches!(
                wparam as u32,
                WM_KEYDOWN | WM_SYSKEYDOWN | WM_KEYUP | WM_SYSKEYUP
            );
            // 仅对 down/up 主事件调用消费，避免无关 wparam 误吃记录
            if is_down_or_up && try_consume_injection(key, is_up) {
                // SAFETY: WH_KEYBOARD_LL 文档允许传入 null hhk,Windows 会沿钩链向后传递
                return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
            }

            let engine = engine_from_hook();
            if let Some(engine) = engine {
                match wparam as u32 {
                    WM_KEYDOWN | WM_SYSKEYDOWN => {
                        engine.on_key_press(key);
                    }
                    WM_KEYUP | WM_SYSKEYUP => engine.on_key_release(key),
                    _ => {}
                }
            }
        }
    }
    // SAFETY: 同上,fall-through 路径必须把事件继续传递给后续钩子,否则会吞掉键盘输入
    CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}

/// 把 wparam + MSLLHOOKSTRUCT 解析为 (按钮, 是否抬起)。仅识别 5 个按钮事件，
/// 移动 / 滚轮 / 双击不映射，调用方应直接转发。
#[cfg(windows)]
fn classify_mouse_event(wparam: u32, mouse_data: u32) -> Option<(MouseButton, bool)> {
    match wparam {
        WM_LBUTTONDOWN => Some((MouseButton::Left, false)),
        WM_LBUTTONUP => Some((MouseButton::Left, true)),
        WM_RBUTTONDOWN => Some((MouseButton::Right, false)),
        WM_RBUTTONUP => Some((MouseButton::Right, true)),
        WM_MBUTTONDOWN => Some((MouseButton::Middle, false)),
        WM_MBUTTONUP => Some((MouseButton::Middle, true)),
        WM_XBUTTONDOWN | WM_XBUTTONUP => {
            // MSLLHOOKSTRUCT.mouseData 高 16 位是 XBUTTON1 / XBUTTON2 标识
            let xbtn = ((mouse_data >> 16) & 0xFFFF) as u16;
            let btn = if xbtn == XBUTTON1 {
                MouseButton::X1
            } else if xbtn == XBUTTON2 {
                MouseButton::X2
            } else {
                return None;
            };
            Some((btn, wparam == WM_XBUTTONUP))
        }
        _ => None,
    }
}

/// WH_MOUSE_LL 低级鼠标钩子回调；与键盘 hook 共用同一消息循环线程。
///
/// # Safety
///
/// 由 Windows 调用：当 `ncode >= 0` 时 `lparam` 指向 Windows 维护的
/// 有效 `MSLLHOOKSTRUCT`，生命周期覆盖本次回调返回前。函数内不持有该指针的延长引用。
#[cfg(windows)]
unsafe extern "system" fn mouse_hook_proc(ncode: i32, wparam: WPARAM, lparam: LPARAM) -> isize {
    if ncode >= 0 {
        // SAFETY: 上文 # Safety 契约保证 ncode>=0 时 lparam 指向有效 MSLLHOOKSTRUCT
        let ms = &*(lparam as *const MSLLHOOKSTRUCT);
        let is_sim_marker = ms.dwExtraInfo == SIM_MARKER;
        if !is_sim_marker {
            if let Some((btn, is_up)) = classify_mouse_event(wparam as u32, ms.mouseData) {
                let key = KeyId::Mouse(btn);
                if try_consume_injection(key, is_up) {
                    // SAFETY: 文档允许 null hhk
                    return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
                }
                if let Some(engine) = engine_from_hook() {
                    if is_up {
                        engine.on_key_release(key);
                    } else {
                        engine.on_key_press(key);
                    }
                }
            }

            // 滚轮触发：每格作为瞬发事件，发 press 后立即发 release
            // Toggle 规则每格切换一次；Hold 规则每格触发一个间隔周期
            if wparam as u32 == WM_MOUSEWHEEL {
                let delta = ((ms.mouseData >> 16) as u16) as i16;
                let btn = if delta > 0 {
                    MouseButton::WheelUp
                } else {
                    MouseButton::WheelDown
                };
                let key = KeyId::Mouse(btn);
                // DD-HID 注入的滚轮通过 PENDING_INJECTIONS 过滤
                if try_consume_injection(key, false) {
                    return CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam);
                }
                if let Some(engine) = engine_from_hook() {
                    engine.on_key_press(key);
                    engine.on_key_release(key);
                }
            }
        }
    }
    // SAFETY: 同上,fall-through 路径必须把事件继续传递给后续钩子
    CallNextHookEx(std::ptr::null_mut(), ncode, wparam, lparam)
}

#[cfg(windows)]
pub fn start_listener(engine: Arc<BurstEngine>) {
    {
        let mut guard = ENGINE_HOOK.write().unwrap_or_else(|e| e.into_inner());
        if guard.as_ref().and_then(|w| w.upgrade()).is_some() {
            error!("start_listener 重复调用：旧引擎仍存活，忽略以防双重 hook");
            return;
        }
        *guard = Some(Arc::downgrade(&engine));
    }
    thread::spawn(move || {
        // SAFETY: 在安装 hook 前记录线程 ID，供 rehook_keyboard() 跨线程投递消息
        HOOK_THREAD_ID.store(unsafe { GetCurrentThreadId() }, Ordering::SeqCst);

        // SAFETY: WH_KEYBOARD_LL 全局钩子允许 hmod=null + dwThreadId=0,Windows
        // 会自行加载本进程模块作为 hook owner;hook_proc 满足 # Safety 契约
        let mut kbd_hook = unsafe {
            SetWindowsHookExW(
                WH_KEYBOARD_LL,
                Some(keyboard_hook_proc),
                std::ptr::null_mut(),
                0,
            )
        };
        if kbd_hook.is_null() {
            error!("安装键盘 hook 失败");
            return;
        }
        info!("键盘 hook 已安装");

        // SAFETY: WH_MOUSE_LL 全局钩子规则与键盘相同
        let mouse_hook = unsafe {
            SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_hook_proc), std::ptr::null_mut(), 0)
        };
        if mouse_hook.is_null() {
            error!("安装鼠标 hook 失败，鼠标按键将无法触发连发");
        } else {
            info!("鼠标 hook 已安装");
        }

        // WH_KEYBOARD_LL / WH_MOUSE_LL 都要求安装线程持续运行消息循环，
        // 否则 Windows 会在超时后移除 hook
        // SAFETY: MSG 是 POD 结构,全 0 是合法初值,GetMessageW 会写入有效字段
        let mut msg = unsafe { std::mem::zeroed::<MSG>() };
        loop {
            // SAFETY: msg 来自上面 zeroed,后续 GetMessageW/Translate/Dispatch
            // 都按 Win32 文档以可变指针写入或只读消费,生命周期不超出本作用域
            let ret = unsafe { GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) };
            if ret == 0 || ret == -1 {
                break;
            }
            // 面板获得焦点时触发：重装键盘 hook 使我们排在 Chromium hook 之后安装（LIFO 优先调用）
            if msg.hwnd.is_null() && msg.message == WM_REHOOK_KEYBOARD {
                if !kbd_hook.is_null() {
                    // SAFETY: kbd_hook 是之前 SetWindowsHookExW 返回的有效句柄
                    unsafe { UnhookWindowsHookEx(kbd_hook) };
                }
                kbd_hook = unsafe {
                    SetWindowsHookExW(
                        WH_KEYBOARD_LL,
                        Some(keyboard_hook_proc),
                        std::ptr::null_mut(),
                        0,
                    )
                };
                if kbd_hook.is_null() {
                    error!("rehook: 键盘 hook 重新安装失败");
                } else {
                    info!("rehook: 键盘 hook 已重新安装");
                }
                continue;
            }
            // SAFETY: msg 是上一步 GetMessageW 写入的合法消息
            unsafe {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
        HOOK_THREAD_ID.store(0, Ordering::SeqCst);
        if !kbd_hook.is_null() {
            // SAFETY: kbd_hook 是上面 SetWindowsHookExW 返回的非空有效句柄
            unsafe { UnhookWindowsHookEx(kbd_hook) };
            info!("键盘 hook 已卸载");
        }
        if !mouse_hook.is_null() {
            // SAFETY: mouse_hook 上面已校验非空
            unsafe { UnhookWindowsHookEx(mouse_hook) };
            info!("鼠标 hook 已卸载");
        }
    });
    info!("连发引擎监听器已启动");
}

#[cfg(not(windows))]
pub fn start_listener(_engine: Arc<BurstEngine>) {
    info!("连发引擎监听器（当前平台暂不支持键盘 hook）");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn rule(id: &str, mode: BurstMode, trigger_key: KeyId, target_key: KeyId) -> BurstRule {
        BurstRule {
            id: id.to_string(),
            enabled: true,
            trigger_key,
            target_key,
            mode,
            stop_key: None,
            interval_ms: 10,
            group: None,
        }
    }

    #[test]
    fn repeated_keydown_does_not_retrigger_global_toggle_before_release() {
        let engine = BurstEngine::new();
        let key = KeyId::Keyboard(0x51);
        engine.set_hotkeys(Hotkeys {
            global_toggle: Some(key),
            ..Default::default()
        });

        engine.on_key_press(key);
        assert!(engine.global_enabled.load(Ordering::SeqCst));

        engine.on_key_press(key);
        assert!(engine.global_enabled.load(Ordering::SeqCst));

        engine.on_key_release(key);
        engine.on_key_press(key);
        assert!(!engine.global_enabled.load(Ordering::SeqCst));
    }

    #[test]
    fn repeated_keydown_calls_panel_toggle_once_until_release() {
        let engine = BurstEngine::new();
        let key = KeyId::Keyboard(0x51);
        let calls = Arc::new(AtomicUsize::new(0));
        let calls_for_cb = calls.clone();
        engine.set_hotkeys(Hotkeys {
            panel_toggle: Some(key),
            ..Default::default()
        });
        engine.set_on_panel_toggle(move || {
            calls_for_cb.fetch_add(1, Ordering::SeqCst);
        });

        engine.on_key_press(key);
        engine.on_key_press(key);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        engine.on_key_release(key);
        engine.on_key_press(key);
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn hold_rule_still_starts_on_first_down_and_stops_on_up() {
        let engine = BurstEngine::new();
        let trigger = KeyId::Keyboard(0x51);
        let target = KeyId::Keyboard(0x45);
        engine.set_rules(vec![rule("hold-q", BurstMode::Hold, trigger, target)]);
        engine.set_global_enabled(true, false);

        engine.on_key_press(trigger);
        assert_eq!(engine.get_active_ids(), vec!["hold-q".to_string()]);

        engine.on_key_press(trigger);
        assert_eq!(engine.get_active_ids(), vec!["hold-q".to_string()]);

        engine.on_key_release(trigger);
        assert!(engine.get_active_ids().is_empty());
    }

    #[test]
    fn toggle_rule_still_toggles_after_release_and_next_down() {
        let engine = BurstEngine::new();
        let trigger = KeyId::Keyboard(0x51);
        let target = KeyId::Keyboard(0x45);
        engine.set_rules(vec![rule("toggle-q", BurstMode::Toggle, trigger, target)]);
        engine.set_global_enabled(true, false);

        engine.on_key_press(trigger);
        assert_eq!(engine.get_active_ids(), vec!["toggle-q".to_string()]);

        engine.on_key_press(trigger);
        assert_eq!(engine.get_active_ids(), vec!["toggle-q".to_string()]);

        engine.on_key_release(trigger);
        engine.on_key_press(trigger);
        assert!(engine.get_active_ids().is_empty());
    }

    #[test]
    fn repeated_mouse_down_is_filtered_until_release() {
        let engine = BurstEngine::new();
        let key = KeyId::Mouse(MouseButton::Left);
        engine.set_hotkeys(Hotkeys {
            global_toggle: Some(key),
            ..Default::default()
        });

        engine.on_key_press(key);
        assert!(engine.global_enabled.load(Ordering::SeqCst));

        engine.on_key_press(key);
        assert!(engine.global_enabled.load(Ordering::SeqCst));

        engine.on_key_release(key);
        engine.on_key_press(key);
        assert!(!engine.global_enabled.load(Ordering::SeqCst));
    }

    #[test]
    fn relay_result_distinguishes_repeated_down_from_physical_acceptance() {
        let engine = BurstEngine::new();
        let key = KeyId::Keyboard(0x51);

        let first = engine.on_key_press_event(key);
        let repeated = engine.on_key_press_event(key);

        assert!(first.accepted_physical);
        assert!(!repeated.accepted_physical);
    }

    #[test]
    fn toggle_group_starts_new_rule_and_stops_old_rule() {
        let engine = BurstEngine::new();
        let mut a = rule(
            "toggle-a",
            BurstMode::Toggle,
            KeyId::Keyboard(0x51),
            KeyId::Keyboard(0x41),
        );
        let mut b = rule(
            "toggle-b",
            BurstMode::Toggle,
            KeyId::Keyboard(0x45),
            KeyId::Keyboard(0x42),
        );
        a.group = Some("g".to_string());
        b.group = Some("g".to_string());
        engine.set_rules(vec![a, b]);
        engine.set_global_enabled(true, false);

        engine.on_key_press(KeyId::Keyboard(0x51));
        engine.on_key_release(KeyId::Keyboard(0x51));
        engine.on_key_press(KeyId::Keyboard(0x45));

        assert_eq!(engine.get_active_ids(), vec!["toggle-b".to_string()]);
    }

    #[test]
    fn stop_resets_physical_ledger_so_next_press_is_accepted() {
        // 安全底线：停止/暂停时复位物理账本，清掉可能残留的幻影按下，
        // 使之后真实按键不会被去重逻辑误吞。
        let engine = BurstEngine::new();
        let key = KeyId::Keyboard(0x60);

        assert!(engine.on_key_press_event(key).accepted_physical);
        // 未释放再次按下：去重判定为按住中，不接受为新物理按下。
        assert!(!engine.on_key_press_event(key).accepted_physical);

        // 停止活动会复位物理账本。
        engine.set_global_enabled(false, false);

        // 账本已复位：同一键再次按下重新被接受为首次物理按下。
        assert!(engine.on_key_press_event(key).accepted_physical);
    }

    #[test]
    fn dedicated_stop_hotkey_fires_even_when_physical_ledger_retains_key() {
        // 安全底线：专用全局停止键 100% 可触发，即便停止键残留在 physical_pressed
        // （注入回灌泄漏 / 未配对释放）导致去重命中，也必须照常停止。
        let engine = BurstEngine::new();
        let toggle = KeyId::Keyboard(0x70);
        let stop = KeyId::Keyboard(0x71);
        engine.set_hotkeys(Hotkeys {
            global_toggle: Some(toggle),
            global_stop: Some(stop),
            ..Default::default()
        });

        engine.set_global_enabled(true, false);
        assert!(engine.global_enabled.load(Ordering::SeqCst));

        // 直接注入幻影项：停止键残留在 physical_pressed（模拟注入回灌泄漏）。
        revive(engine.physical_pressed.lock()).insert(stop);

        // 按下停止键：is_new=false（幻影命中去重），但停止必须 100% 触发。
        let result = engine.on_key_press_event(stop);
        assert!(!result.accepted_physical); // 去重命中：账本里已有该键
        assert!(result.handled); // 但停止照常触发
        assert!(!engine.global_enabled.load(Ordering::SeqCst));
    }

    #[test]
    fn toggle_with_separate_stop_key_only_stops_on_stop_key() {
        // 边界：Toggle 配独立停止键时，再按触发键不停止，只有按停止键才停。
        let engine = BurstEngine::new();
        let trigger = KeyId::Keyboard(0x51); // Q
        let stop = KeyId::Keyboard(0x58); // X
        let mut r = rule("t", BurstMode::Toggle, trigger, KeyId::Keyboard(0x45));
        r.stop_key = Some(stop);
        engine.set_rules(vec![r]);
        engine.set_global_enabled(true, false);

        engine.on_key_press(trigger);
        assert_eq!(engine.get_active_ids(), vec!["t".to_string()]);

        // 再按触发键：停止键是独立的 X，不停止。
        engine.on_key_release(trigger);
        engine.on_key_press(trigger);
        assert_eq!(engine.get_active_ids(), vec!["t".to_string()]);

        // 按下独立停止键才停止。
        engine.on_key_release(trigger);
        engine.on_key_press(stop);
        assert!(engine.get_active_ids().is_empty());
    }

    #[test]
    fn toggle_group_displacement_resets_state_so_old_rule_can_restart() {
        // 边界（对应历史 toggle 双状态竞态）：A 被同组 B 顶替后，其 toggle 状态必须复位，
        // 之后再按 A 的触发键能重新开启 A（并反过来顶替 B）。
        let engine = BurstEngine::new();
        let mut a = rule(
            "a",
            BurstMode::Toggle,
            KeyId::Keyboard(0x51),
            KeyId::Keyboard(0x41),
        );
        let mut b = rule(
            "b",
            BurstMode::Toggle,
            KeyId::Keyboard(0x45),
            KeyId::Keyboard(0x42),
        );
        a.group = Some("g".to_string());
        b.group = Some("g".to_string());
        engine.set_rules(vec![a, b]);
        engine.set_global_enabled(true, false);

        engine.on_key_press(KeyId::Keyboard(0x51)); // a 开
        assert_eq!(engine.get_active_ids(), vec!["a".to_string()]);
        engine.on_key_release(KeyId::Keyboard(0x51));

        engine.on_key_press(KeyId::Keyboard(0x45)); // b 开，a 被顶替
        assert_eq!(engine.get_active_ids(), vec!["b".to_string()]);
        engine.on_key_release(KeyId::Keyboard(0x45));

        engine.on_key_press(KeyId::Keyboard(0x51)); // 再按 Q：a 重新开启并顶替 b
        assert_eq!(engine.get_active_ids(), vec!["a".to_string()]);
    }

    #[test]
    fn rule_does_not_start_while_global_disabled() {
        // 边界：全局开关未启用时，按下触发键不应启动任何连发。
        let engine = BurstEngine::new();
        let trigger = KeyId::Keyboard(0x51);
        engine.set_rules(vec![rule(
            "h",
            BurstMode::Hold,
            trigger,
            KeyId::Keyboard(0x45),
        )]);
        // 故意不调用 set_global_enabled(true)
        engine.on_key_press(trigger);
        assert!(engine.get_active_ids().is_empty());
    }

    #[test]
    fn enabling_after_shutdown_is_rejected() {
        // 边界（A2）：关停后再 set_global_enabled(true)（等价于关停期热键开启）必须被拒绝，
        // 不得把 lifecycle 复活成 Running、也不得把 global_enabled 残留为 true（调度器已拆，避免割裂态）。
        let engine = BurstEngine::new();
        engine.shutdown();
        assert_eq!(engine.lifecycle(), EngineLifecycle::Shutdown);

        engine.set_global_enabled(true, false);

        assert!(!engine.global_enabled.load(Ordering::SeqCst));
        assert_eq!(engine.lifecycle(), EngineLifecycle::Shutdown);
    }

    #[test]
    fn set_rules_swaps_snapshot_so_removed_rule_no_longer_triggers() {
        // 边界（C3）：set_rules 换规则后，旧规则触发键不再启动连发（快照已换）；新规则触发键正常启动。
        // 配合「先换快照后停活动」的顺序，杜绝已删规则在新 generation 上被旧快照启动、释放却查不到而残留连发。
        let engine = BurstEngine::new();
        let old_trigger = KeyId::Keyboard(0x51);
        let new_trigger = KeyId::Keyboard(0x45);
        engine.set_rules(vec![rule(
            "old",
            BurstMode::Hold,
            old_trigger,
            KeyId::Keyboard(0x41),
        )]);
        engine.set_global_enabled(true, false);

        // 换成只含 new 规则的集合；global 仍为 on，lifecycle 经 stop_runtime_activity 恢复 Running。
        engine.set_rules(vec![rule(
            "new",
            BurstMode::Hold,
            new_trigger,
            KeyId::Keyboard(0x42),
        )]);

        engine.on_key_press(old_trigger);
        assert!(engine.get_active_ids().is_empty());

        engine.on_key_press(new_trigger);
        assert_eq!(engine.get_active_ids(), vec!["new".to_string()]);
    }

    #[test]
    fn backend_switch_blocks_rule_start_until_finished() {
        // 边界（#1/#2）：后端切换窗口内物理触发不得启动规则（否则目标键 down 走旧后端、up 走新
        // 后端会卡键）；切换结束后恢复正常启动。
        let engine = BurstEngine::new();
        let trigger = KeyId::Keyboard(0x51);
        engine.set_rules(vec![rule(
            "h",
            BurstMode::Hold,
            trigger,
            KeyId::Keyboard(0x41),
        )]);
        engine.set_global_enabled(true, false);

        engine.begin_backend_switch();
        engine.on_key_press(trigger);
        assert!(engine.get_active_ids().is_empty());
        engine.on_key_release(trigger);
        engine.end_backend_switch();

        engine.on_key_press(trigger);
        assert_eq!(engine.get_active_ids(), vec!["h".to_string()]);
    }

    #[test]
    fn global_stop_during_backend_switch_is_preserved() {
        // 边界（#2）：切换窗口内用户按下全局停止后，切换流程不得把开关重新打开——切换全程不触碰
        // global_enabled，故停止得以保留。
        let engine = BurstEngine::new();
        let trigger = KeyId::Keyboard(0x51);
        engine.set_rules(vec![rule(
            "h",
            BurstMode::Hold,
            trigger,
            KeyId::Keyboard(0x41),
        )]);
        engine.set_global_enabled(true, false);

        engine.begin_backend_switch();
        // 模拟切换期间用户的停止（热键路径最终也调用 set_global_enabled(false)）。
        engine.set_global_enabled(false, false);
        engine.end_backend_switch();

        assert!(!engine.global_enabled.load(Ordering::SeqCst));
        engine.on_key_press(trigger);
        assert!(engine.get_active_ids().is_empty());
    }
}
