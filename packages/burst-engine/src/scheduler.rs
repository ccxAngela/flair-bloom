#[cfg(test)]
use crate::output_queue::hold_duration;
use crate::output_queue::{OutputQueue, QueueAction};
use qzh_profile::key_id::KeyId;
use qzh_profile::profile::{BurstRule, MIN_EFFECTIVE_INTERVAL_MS};
use std::collections::{hash_map::Entry, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};
use tracing::{error, warn};
use win_input::{DispatchResult, InputEvent};

const ACK_TIMEOUT: Duration = Duration::from_millis(500);
const RELEASE_RETRY: Duration = Duration::from_millis(50);

#[derive(Debug)]
struct ScheduledRule {
    source_id: Arc<str>,
    queue: OutputQueue,
    generation: u64,
    one_shot: bool,
    run_id: u64,
    throttle_at: Instant,
    paused_throttle: Option<Duration>,
}

impl ScheduledRule {
    fn deadline(&self) -> Option<Instant> {
        self.queue.deadline().map(|deadline| {
            if self.queue.held_key().is_some() {
                deadline
            } else {
                deadline.max(self.throttle_at)
            }
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleProgress {
    pub rule_id: String,
    pub step_index: usize,
    pub step_count: usize,
    pub completed_cycles: u64,
    pub paused: bool,
    pub error: Option<String>,
    pub run_id: u64,
    pub lateness_ms: u64,
    pub max_lateness_ms: u64,
}

type ProgressMap = Arc<Mutex<HashMap<String, RuleProgress>>>;
type Failure = Arc<Mutex<Option<String>>>;

fn report_failure(failure: &Failure, progress: &ProgressMap, message: &str) {
    let message = {
        let mut failure = failure.lock().unwrap_or_else(|e| e.into_inner());
        failure.get_or_insert_with(|| message.to_string()).clone()
    };
    for position in progress
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values_mut()
    {
        position.error = Some(message.clone());
    }
}

enum SchedulerCommand {
    Start {
        rule: Arc<BurstRule>,
        generation: u64,
        run_id: u64,
        once: bool,
    },
    /// 一次性点按一条规则的目标键（滚轮 Hold）。与 [`SchedulerCommand::Start`] 不同：
    /// 不按 `rule.id` 去重，每次都用唯一内部 ID 排程，故同一规则的多格滚轮各点按一次。
    TapOnce {
        rule: Arc<BurstRule>,
        generation: u64,
    },
    Stop {
        rule_id: String,
        generation: u64,
        ack: Option<Sender<()>>,
    },
    Pause {
        rule_id: String,
        generation: u64,
    },
    Resume {
        rule_id: String,
        generation: u64,
    },
    StopAll {
        generation: u64,
        ack: Option<Sender<bool>>,
    },
    Shutdown {
        generation: u64,
        ack: Sender<bool>,
    },
}

pub struct SchedulerHandle {
    tx: Sender<SchedulerCommand>,
    join: Mutex<Option<JoinHandle<()>>>,
    hp_degraded: Arc<AtomicBool>,
    command_waker: platform_wait::CommandWaker,
    progress: ProgressMap,
    next_run: AtomicU64,
    failure: Failure,
}

pub(crate) type PhysicalKeys = Arc<Mutex<HashSet<KeyId>>>;
pub(crate) type SimulatedKeys = Arc<Mutex<HashMap<KeyId, usize>>>;

pub(crate) trait EventDispatcher: Send + Sync {
    fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult>;
}

struct WinInputDispatcher;

impl EventDispatcher for WinInputDispatcher {
    fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
        inject(events)
    }
}

/// 引擎唯一的真注入出口。单测里 `BurstEngine::new()` 也会走到这里，真注入会把测试规则的目标键
/// （E、A、B…）按 10 ms 连发进当前有焦点的窗口——跑测试时正在打字的输入框首当其冲。所以测试
/// 构建一律当作发送成功、什么也不发；真实注入链路由默认 `#[ignore]` 的 `smoke_tests` 直接调
/// `win_input` 验证。
fn inject(events: &[InputEvent]) -> Vec<DispatchResult> {
    #[cfg(not(test))]
    {
        win_input::key_events(events)
    }
    #[cfg(test)]
    {
        vec![DispatchResult::Sent; events.len()]
    }
}

#[derive(Default)]
pub(crate) struct SchedulerStats {
    delay_ns: Mutex<Vec<u128>>,
    /// 每批 dispatch 的「单事件平均注入耗时」（ns）。这是下游背压唯一可观测的代理：
    /// LL hook 链 / RIT / 前台输入队列拥塞时 SendInput/驱动注入会变慢，此值随之上升。
    dispatch_cost_ns: Mutex<Vec<u128>>,
    sent_events: AtomicU64,
    failed_events: AtomicU64,
    batches: AtomicU64,
}

impl SchedulerStats {
    fn record_delay(&self, delay: Duration) {
        self.delay_ns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(delay.as_nanos());
    }

    fn record_dispatch_cost(&self, per_event: Duration) {
        self.dispatch_cost_ns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(per_event.as_nanos());
    }

    fn record_results(&self, results: &[DispatchResult]) {
        self.batches.fetch_add(1, Ordering::Relaxed);
        let sent = results.iter().filter(|r| r.was_sent()).count() as u64;
        let failed = results
            .iter()
            .filter(|r| matches!(r, DispatchResult::Failed))
            .count() as u64;
        self.sent_events.fetch_add(sent, Ordering::Relaxed);
        self.failed_events.fetch_add(failed, Ordering::Relaxed);
    }

    pub(crate) fn snapshot(&self) -> SchedulerStatsSnapshot {
        let delays = self
            .delay_ns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let dispatch_cost = self
            .dispatch_cost_ns
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        SchedulerStatsSnapshot {
            delay_ns: delays,
            dispatch_cost_ns: dispatch_cost,
            sent_events: self.sent_events.load(Ordering::Relaxed),
            failed_events: self.failed_events.load(Ordering::Relaxed),
            batches: self.batches.load(Ordering::Relaxed),
        }
    }
}

pub(crate) struct SchedulerStatsSnapshot {
    pub delay_ns: Vec<u128>,
    pub dispatch_cost_ns: Vec<u128>,
    pub sent_events: u64,
    pub failed_events: u64,
    pub batches: u64,
}

impl SchedulerHandle {
    pub fn start(simulated_keys: SimulatedKeys) -> Self {
        Self::start_with_dispatcher(Arc::new(WinInputDispatcher), None, simulated_keys)
    }

    pub(crate) fn start_with_dispatcher(
        dispatcher: Arc<dyn EventDispatcher>,
        stats: Option<Arc<SchedulerStats>>,
        simulated_keys: SimulatedKeys,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let hp_degraded = Arc::new(AtomicBool::new(false));
        let worker_degraded = hp_degraded.clone();
        let command_waker = platform_wait::CommandWaker::new(&hp_degraded);
        let worker_waker = command_waker.clone();
        let progress = ProgressMap::default();
        let worker_progress = progress.clone();
        let failure = Failure::default();
        let worker_failure = failure.clone();
        let failure_progress = progress.clone();
        // panic 兜底：worker 栈一旦展开，target_holds 随之丢失，靠 simulated_keys 账本
        // 释放所有已按下的模拟键，避免按键卡死（连发工具最差故障）。
        let panic_fallback = simulated_keys.clone();
        let join = thread::spawn(move || {
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                worker_loop(
                    rx,
                    worker_degraded,
                    worker_waker,
                    dispatcher,
                    stats,
                    simulated_keys,
                    worker_progress,
                )
            }));
            if outcome.is_err() {
                report_failure(
                    &worker_failure,
                    &failure_progress,
                    "调度线程异常，输出已停止，请重启应用",
                );
                release_simulated_keys(&panic_fallback);
                error!("调度器 worker 线程 panic，已按账本兜底释放模拟按键");
            }
        });
        Self {
            tx,
            join: Mutex::new(Some(join)),
            hp_degraded,
            command_waker,
            progress,
            next_run: AtomicU64::new(1),
            failure,
        }
    }

    pub fn start_rule(&self, rule: Arc<BurstRule>, generation: u64) {
        self.start_with_policy(rule, generation, false);
    }

    fn start_with_policy(&self, rule: Arc<BurstRule>, generation: u64, once: bool) {
        let failure = self.failure();
        let run_id = {
            let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(current) = progress.get(&rule.id).filter(|p| p.error.is_none()) {
                current.run_id
            } else {
                let run_id = self.next_run.fetch_add(1, Ordering::Relaxed);
                progress.insert(
                    rule.id.clone(),
                    RuleProgress {
                        rule_id: rule.id.clone(),
                        step_index: 0,
                        step_count: rule.steps.len(),
                        completed_cycles: 0,
                        paused: false,
                        error: failure.clone(),
                        run_id,
                        lateness_ms: 0,
                        max_lateness_ms: 0,
                    },
                );
                run_id
            }
        };
        if failure.is_some() {
            return;
        }
        self.send(SchedulerCommand::Start {
            rule,
            generation,
            run_id,
            once,
        });
    }

    pub fn tap_once(&self, rule: Arc<BurstRule>, generation: u64) {
        self.send(SchedulerCommand::TapOnce { rule, generation });
    }

    pub fn stop_rule(&self, rule_id: String, generation: u64) {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if progress.get(&rule_id).is_some_and(|p| p.error.is_none()) {
            progress.remove(&rule_id);
        }
        drop(progress);
        self.send(SchedulerCommand::Stop {
            rule_id,
            generation,
            ack: None,
        });
    }

    pub fn stop_all_async(&self, generation: u64) {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        self.send(SchedulerCommand::StopAll {
            generation,
            ack: None,
        });
    }

    pub fn stop_all_blocking(&self, generation: u64) -> bool {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let (tx, rx) = mpsc::channel();
        if !self.send(SchedulerCommand::StopAll {
            generation,
            ack: Some(tx),
        }) {
            return false;
        }
        rx.recv_timeout(ACK_TIMEOUT).unwrap_or(false)
    }

    pub fn shutdown_blocking(&self, generation: u64) -> bool {
        let (tx, rx) = mpsc::channel();
        if !self.send(SchedulerCommand::Shutdown {
            generation,
            ack: tx,
        }) {
            return false;
        }
        let acked = rx.recv_timeout(ACK_TIMEOUT).unwrap_or(false);
        if let Some(join) = self.join.lock().unwrap_or_else(|e| e.into_inner()).take() {
            let _ = join.join();
        }
        acked
    }

    pub fn hp_degraded(&self) -> bool {
        self.hp_degraded.load(Ordering::SeqCst)
    }

    fn send(&self, command: SchedulerCommand) -> bool {
        if self.tx.send(command).is_err() {
            report_failure(
                &self.failure,
                &self.progress,
                "调度线程已退出，输出已停止，请重启应用",
            );
            return false;
        }
        self.command_waker.wake();
        true
    }
}

impl Drop for SchedulerHandle {
    fn drop(&mut self) {
        let _ = self.shutdown_blocking(u64::MAX);
    }
}

/// 引擎依赖的调度器命令接口。抽成 trait 是为了在测试里注入「命令录制替身」，
/// 对「合成物理按键 → 引擎 → 调度命令（含 generation）」整条管线做确定性断言——
/// 真实 [`SchedulerHandle`] 跑在真线程 + waitable timer 上，无法直接确定性测命令时序。
pub(crate) trait Scheduler: Send + Sync {
    fn start_rule(&self, rule: Arc<BurstRule>, generation: u64);
    fn tap_once(&self, rule: Arc<BurstRule>, generation: u64);
    fn stop_rule(&self, rule_id: String, generation: u64);
    fn stop_all_async(&self, generation: u64);
    fn stop_all_blocking(&self, generation: u64) -> bool;
    fn shutdown_blocking(&self, generation: u64) -> bool;
    fn hp_degraded(&self) -> bool;
    fn pause_rule(&self, rule_id: String, generation: u64);
    fn resume_rule(&self, rule_id: String, generation: u64);
    fn play_once(&self, rule: Arc<BurstRule>, generation: u64);
    fn failure(&self) -> Option<String> {
        None
    }
    fn progress(&self) -> Vec<RuleProgress> {
        Vec::new()
    }
}

impl Scheduler for SchedulerHandle {
    fn failure(&self) -> Option<String> {
        self.failure
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }
    fn resume_rule(&self, rule_id: String, generation: u64) {
        self.send(SchedulerCommand::Resume {
            rule_id,
            generation,
        });
    }
    fn play_once(&self, rule: Arc<BurstRule>, generation: u64) {
        self.start_with_policy(rule, generation, true);
    }
    fn pause_rule(&self, rule_id: String, generation: u64) {
        self.send(SchedulerCommand::Pause {
            rule_id,
            generation,
        });
    }
    fn progress(&self) -> Vec<RuleProgress> {
        self.progress
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .values()
            .cloned()
            .collect()
    }
    fn start_rule(&self, rule: Arc<BurstRule>, generation: u64) {
        SchedulerHandle::start_rule(self, rule, generation);
    }
    fn tap_once(&self, rule: Arc<BurstRule>, generation: u64) {
        SchedulerHandle::tap_once(self, rule, generation);
    }
    fn stop_rule(&self, rule_id: String, generation: u64) {
        SchedulerHandle::stop_rule(self, rule_id, generation);
    }
    fn stop_all_async(&self, generation: u64) {
        SchedulerHandle::stop_all_async(self, generation);
    }
    fn stop_all_blocking(&self, generation: u64) -> bool {
        SchedulerHandle::stop_all_blocking(self, generation)
    }
    fn shutdown_blocking(&self, generation: u64) -> bool {
        SchedulerHandle::shutdown_blocking(self, generation)
    }
    fn hp_degraded(&self) -> bool {
        SchedulerHandle::hp_degraded(self)
    }
}

fn worker_loop(
    rx: Receiver<SchedulerCommand>,
    hp_degraded: Arc<AtomicBool>,
    command_waker: platform_wait::CommandWaker,
    dispatcher: Arc<dyn EventDispatcher>,
    stats: Option<Arc<SchedulerStats>>,
    simulated_keys: SimulatedKeys,
    progress: ProgressMap,
) {
    let mut worker = SchedulerWorker {
        rules: HashMap::new(),
        target_holds: HashMap::new(),
        current_generation: 0,
        dispatcher,
        stats,
        simulated_keys,
        min_interval_ms: MIN_EFFECTIVE_INTERVAL_MS,
        tap_seq: 0,
        progress,
        clock: |_| Instant::now(),
        cleanup_at: None,
    };
    let mut wait = platform_wait::WaitContext::new(command_waker, hp_degraded);

    loop {
        let deadline = worker.next_deadline();
        let command = match wait.recv(&rx, deadline) {
            platform_wait::WaitOutcome::Command(command) => Some(command),
            platform_wait::WaitOutcome::Timeout => None,
            platform_wait::WaitOutcome::Disconnected => {
                worker.cleanup_all();
                break;
            }
        };

        if let Some(command) = command {
            if worker.handle_command(command) {
                break;
            }
            while let Ok(command) = rx.try_recv() {
                if worker.handle_command(command) {
                    return;
                }
            }
        }

        worker.process_due(Instant::now());
    }
}

struct SchedulerWorker {
    /// 规则 ID 用 `Arc<str>` 而非 `String`：owner 登记到 `target_holds` 时只做引用计数
    /// 克隆，避免热路径（`process_due` 每拍 acquire/release）的 `String` 堆分配。
    rules: HashMap<Arc<str>, ScheduledRule>,
    /// 已取得目标键的规则；残留账本也会阻止重新 down，避免释放失败后合并不同步骤。
    target_holds: HashMap<KeyId, HashSet<Arc<str>>>,
    current_generation: u64,
    dispatcher: Arc<dyn EventDispatcher>,
    stats: Option<Arc<SchedulerStats>>,
    simulated_keys: SimulatedKeys,
    /// 注入周期「基础」下限（生产取 [`MIN_EFFECTIVE_INTERVAL_MS`]，测试构造器设 1 不限速）。
    /// 每条规则的有效下限 = 此值 × 当前活跃规则数（总并发限速，详见 `process_due`）：管线可持续
    /// 的「总」注入速率有上限，多条规则同时连发按规则数等分，避免叠加冲破天花板导致「收不住」。
    /// 只拉长拍间间隔、不影响 `hold` 点按时长。
    min_interval_ms: u32,
    /// 一次性点按（`TapOnce`）的内部自增序号，用于生成唯一排程 ID，使同一规则的多次点按互不去重。
    tap_seq: u64,
    progress: ProgressMap,
    clock: fn(Instant) -> Instant,
    cleanup_at: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AcquireOutcome {
    Acquired,
    Shared,
}

impl SchedulerWorker {
    fn handle_command(&mut self, command: SchedulerCommand) -> bool {
        match command {
            SchedulerCommand::Start {
                rule,
                generation,
                run_id,
                once,
            } => {
                if generation == self.current_generation {
                    let id: Arc<str> = Arc::from(rule.id.as_str());
                    if rule.validate().is_ok() {
                        self.insert_scheduled(id, rule, generation, once, run_id);
                    }
                }
                false
            }
            SchedulerCommand::TapOnce { rule, generation } => {
                if generation == self.current_generation {
                    // 唯一内部 ID：不与 rule.id 冲突，也使同一规则的多格滚轮各排一条、互不去重。
                    self.tap_seq = self.tap_seq.wrapping_add(1);
                    let id: Arc<str> = Arc::from(format!("\u{0}tap#{}", self.tap_seq).as_str());
                    self.insert_scheduled(id, rule, generation, true, 0);
                }
                false
            }
            SchedulerCommand::Pause {
                rule_id,
                generation,
            } => {
                if generation == self.current_generation {
                    self.pause_rule(&rule_id, Instant::now());
                }
                false
            }
            SchedulerCommand::Resume {
                rule_id,
                generation,
            } => {
                if generation == self.current_generation {
                    // 失败或停止后实例已移除；恢复不能新建队列或清掉失败回报。
                    self.resume_rule(&rule_id, Instant::now());
                }
                false
            }
            SchedulerCommand::Stop {
                rule_id,
                generation,
                ack,
            } => {
                if generation == self.current_generation {
                    self.stop_rule(&rule_id);
                }
                if let Some(ack) = ack {
                    let _ = ack.send(());
                }
                false
            }
            SchedulerCommand::StopAll { generation, ack } => {
                let released = if generation >= self.current_generation {
                    self.current_generation = generation;
                    self.cleanup_all();
                    self.simulated_keys
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty()
                } else {
                    // 旧停止不能以新运行的按键为失败依据，触发兜底会误松新按键。
                    true
                };
                if let Some(ack) = ack {
                    let _ = ack.send(released);
                }
                false
            }
            SchedulerCommand::Shutdown { generation, ack } => {
                self.current_generation = self.current_generation.max(generation);
                self.cleanup_all();
                let _ = ack.send(
                    self.simulated_keys
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .is_empty(),
                );
                true
            }
        }
    }

    /// `one_shot=true` 在完整队列执行一次后自移除，供试播与一步滚轮 Hold 共用。
    /// 以 `entry().or_insert` 去重：普通规则按 `rule.id` 幂等，一次性点按用唯一 ID
    /// 故每次都新增。
    fn insert_scheduled(
        &mut self,
        id: Arc<str>,
        rule: Arc<BurstRule>,
        generation: u64,
        one_shot: bool,
        run_id: u64,
    ) {
        let now = (self.clock)(Instant::now());
        let Ok(queue) = OutputQueue::new(rule.steps.clone(), now) else {
            return;
        };
        self.rules.entry(id).or_insert(ScheduledRule {
            source_id: Arc::from(rule.id.as_str()),
            queue,
            generation,
            one_shot,
            run_id,
            throttle_at: now,
            paused_throttle: None,
        });
    }

    fn stop_rule(&mut self, rule_id: &str) {
        let Some(rule) = self.rules.remove(rule_id) else {
            return;
        };
        if let Some(key) = rule.queue.held_key() {
            let event = Self::release_owner(&mut self.target_holds, key, rule_id);
            if !self.dispatch_events(event.into_iter().collect()) {
                self.dispatch_events(vec![InputEvent::up(key)]);
            }
        }
    }

    fn cleanup_all(&mut self) {
        self.rules.clear();
        let events = self.release_all_target_holds();
        self.target_holds.clear();
        self.dispatch_events(events);
        // 释放 target_holds 后再按账本兜底对账，补发任何「曾注入 down、但其 up 注入失败」
        // 而残留按下态的目标键，杜绝卡键（按键工具最差故障）。
        self.reconcile_simulated_ledger();
        self.refresh_cleanup_deadline((self.clock)(Instant::now()));
    }

    /// 兜底对账：把账本里仍记为「按下」的目标键补发一次 up。
    /// 释放失败或发送结果未知时仍可能保持按下；只有成功 up 才能清除记录。
    fn reconcile_simulated_ledger(&self) {
        let stuck: Vec<KeyId> = {
            let simulated = self
                .simulated_keys
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            if simulated.is_empty() {
                return;
            }
            simulated
                .keys()
                .filter(|key| !self.target_holds.contains_key(key))
                .copied()
                .collect()
        };
        let events: Vec<InputEvent> = stuck.into_iter().map(InputEvent::up).collect();
        self.dispatch_events(events);
    }

    fn refresh_cleanup_deadline(&mut self, now: Instant) {
        let has_orphan = self
            .simulated_keys
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .any(|key| !self.target_holds.contains_key(key));
        if has_orphan {
            self.cleanup_at.get_or_insert(now + RELEASE_RETRY);
        } else {
            self.cleanup_at = None;
        }
    }

    fn next_deadline(&self) -> Option<Instant> {
        self.rules
            .values()
            .filter_map(ScheduledRule::deadline)
            .chain(self.cleanup_at)
            .min()
    }

    fn publish(&self, rule: &ScheduledRule, error: Option<String>) {
        let mut progress = self.progress.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(current) = progress
            .get_mut(rule.source_id.as_ref())
            .filter(|p| p.run_id == rule.run_id)
        {
            current.step_index = rule.queue.step_index();
            current.step_count = rule.queue.step_count();
            current.completed_cycles = rule.queue.completed_cycles();
            current.paused = rule.queue.paused_remaining().is_some();
            let (last, maximum) = rule.queue.lateness();
            current.lateness_ms = last.as_millis().min(u128::from(u64::MAX)) as u64;
            current.max_lateness_ms = maximum.as_millis().min(u128::from(u64::MAX)) as u64;
            current.error = error;
        }
    }

    fn pause_rule(&mut self, id: &str, now: Instant) {
        let Some(mut rule) = self.rules.remove(id) else {
            return;
        };
        if rule.queue.paused_remaining().is_none() {
            if let Some(key) = rule.queue.held_key() {
                let event = Self::release_owner(&mut self.target_holds, key, id);
                if !self.dispatch_events(event.into_iter().collect()) {
                    self.dispatch_events(vec![InputEvent::up(key)]);
                    self.publish(
                        &rule,
                        Some(format!(
                            "第 {} 步暂停释放失败，已停止",
                            rule.queue.step_index() + 1
                        )),
                    );
                    return;
                }
            }
            rule.paused_throttle = Some(rule.throttle_at.saturating_duration_since(now));
            rule.queue.pause(now);
        }
        self.publish(&rule, None);
        self.rules.insert(Arc::from(id), rule);
    }

    fn resume_rule(&mut self, id: &str, now: Instant) {
        if let Some(rule) = self.rules.get_mut(id) {
            rule.queue.resume(now);
            if let Some(remaining) = rule.paused_throttle.take() {
                rule.throttle_at = now + remaining;
            }
        }
        if let Some(rule) = self.rules.get(id) {
            self.publish(rule, None);
        }
    }

    fn process_due(&mut self, now: Instant) {
        if self.cleanup_at.is_some_and(|deadline| deadline <= now) {
            self.cleanup_at = None;
            self.reconcile_simulated_ledger();
        }
        let active_count = self
            .rules
            .values()
            .filter(|r| r.generation == self.current_generation && r.queue.deadline().is_some())
            .count()
            .max(1) as u32;
        let throttle =
            Duration::from_millis(u64::from(self.min_interval_ms.saturating_mul(active_count)));
        let mut due: Vec<_> = self
            .rules
            .iter()
            .filter_map(|(id, rule)| {
                let deadline = rule.deadline()?;
                // 重试和节流不重置排队年龄，避免同键高负载时旧步骤一直被新一轮挤走。
                (deadline <= now).then_some((
                    id.clone(),
                    rule.queue.deadline().unwrap_or(deadline),
                    rule.queue.held_key().is_none(),
                ))
            })
            .collect();
        due.sort_by(|a, b| (a.2, a.1, &a.0).cmp(&(b.2, b.1, &b.0)));
        for (id, _, _) in due {
            let Some(mut rule) = self.rules.remove(&id) else {
                continue;
            };
            if rule.generation != self.current_generation {
                if let Some(key) = rule.queue.held_key() {
                    let event = Self::release_owner(&mut self.target_holds, key, &id);
                    self.dispatch_events(event.into_iter().collect());
                }
                continue;
            }
            let tick = (self.clock)(now);
            let Some(action) = rule.queue.peek(tick) else {
                self.rules.insert(id, rule);
                continue;
            };
            let key = action.key();
            if !action.is_release() && self.target_holds.contains_key(&key) {
                rule.throttle_at = tick + Duration::from_millis(1);
                self.rules.insert(id, rule);
                continue;
            }
            if !action.is_release()
                && self
                    .simulated_keys
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains_key(&key)
            {
                // 所有者消失不代表 up 成功；先修复残留按下态，下一拍才允许新的完整点按。
                if self.cleanup_at.is_none_or(|deadline| deadline <= tick) {
                    self.dispatch_events(vec![InputEvent::up(key)]);
                    self.cleanup_at = Some(tick + RELEASE_RETRY);
                }
                rule.throttle_at = tick + RELEASE_RETRY;
                self.rules.insert(id, rule);
                continue;
            }
            if let Some(stats) = &self.stats {
                if let Some(deadline) = rule.queue.deadline() {
                    stats.record_delay(tick.saturating_duration_since(deadline));
                }
            }
            let events = match action {
                QueueAction::Press(_) => {
                    Self::acquire_owner(&mut self.target_holds, key, &id);
                    vec![InputEvent::down(key)]
                }
                QueueAction::Tap(_) => {
                    Self::acquire_owner(&mut self.target_holds, key, &id);
                    let mut events = vec![InputEvent::down(key)];
                    events.extend(Self::release_owner(&mut self.target_holds, key, &id));
                    events
                }
                QueueAction::Release(_) => Self::release_owner(&mut self.target_holds, key, &id)
                    .into_iter()
                    .collect(),
            };
            let t0 = Instant::now();
            let event_count = events.len();
            let succeeded = self.dispatch_events(events);
            let completed_at = (self.clock)(now);
            if let Some(stats) = &self.stats {
                if event_count > 0 {
                    stats.record_dispatch_cost(t0.elapsed() / event_count as u32);
                }
            }
            let acknowledged = succeeded && rule.queue.acknowledge(action, completed_at).is_ok();
            if !acknowledged {
                let release = Self::release_owner(&mut self.target_holds, key, &id);
                self.dispatch_events(release.into_iter().collect());
                if self
                    .simulated_keys
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .contains_key(&key)
                {
                    self.dispatch_events(vec![InputEvent::up(key)]);
                }
                self.publish(
                    &rule,
                    Some(format!(
                        "第 {} 步输入发送失败，已停止",
                        rule.queue.step_index() + 1
                    )),
                );
                continue;
            }
            if !action.is_release() {
                rule.throttle_at = completed_at + throttle;
            }
            self.publish(&rule, None);
            if !(rule.one_shot && rule.queue.completed_cycles() > 0) {
                self.rules.insert(id, rule);
            }
        }
        self.refresh_cleanup_deadline((self.clock)(now));
    }

    fn acquire_owner(
        target_holds: &mut HashMap<KeyId, HashSet<Arc<str>>>,
        key: KeyId,
        owner: &Arc<str>,
    ) -> AcquireOutcome {
        match target_holds.entry(key) {
            Entry::Occupied(mut e) => {
                e.get_mut().insert(owner.clone());
                AcquireOutcome::Shared
            }
            Entry::Vacant(e) => {
                e.insert(HashSet::new()).insert(owner.clone());
                AcquireOutcome::Acquired
            }
        }
    }

    fn release_owner(
        target_holds: &mut HashMap<KeyId, HashSet<Arc<str>>>,
        key: KeyId,
        owner: &str,
    ) -> Option<InputEvent> {
        let owners = target_holds.get_mut(&key)?;
        owners.remove(owner);
        if !owners.is_empty() {
            None
        } else {
            target_holds.remove(&key);
            Some(InputEvent::up(key))
        }
    }

    fn release_all_target_holds(&mut self) -> Vec<InputEvent> {
        self.target_holds
            .drain()
            .map(|(key, _)| InputEvent::up(key))
            .collect()
    }

    fn dispatch_events(&self, events: Vec<InputEvent>) -> bool {
        if events.is_empty() {
            return true;
        }
        let results = dispatch_tracked(self.dispatcher.as_ref(), &self.simulated_keys, &events);
        let succeeded = results.len() == events.len()
            && events.iter().zip(&results).all(|(event, result)| {
                result.was_sent()
                    || (event.key.is_wheel() && event.is_up && *result == DispatchResult::Noop)
            });
        if let Some(stats) = &self.stats {
            stats.record_results(&results);
        }
        for (event, result) in events.iter().zip(results) {
            if result == DispatchResult::Failed {
                warn!("输入注入失败: key={:?} is_up={}", event.key, event.is_up);
            }
        }
        succeeded
    }
}

fn dispatch_tracked(
    dispatcher: &dyn EventDispatcher,
    keys: &SimulatedKeys,
    events: &[InputEvent],
) -> Vec<DispatchResult> {
    // 注入与回报落账必须串行，避免兜底释放与新 down 交错后误删新状态；钩子不获取此锁。
    let mut simulated = keys.lock().unwrap_or_else(|e| e.into_inner());
    let previous: HashMap<_, _> = events
        .iter()
        .filter(|event| !event.is_up)
        .filter_map(|event| simulated.get(&event.key).map(|count| (event.key, *count)))
        .collect();
    for event in events
        .iter()
        .filter(|event| !event.is_up && !event.key.is_wheel())
    {
        // 发送函数 panic 时结果未知，宁可保留待释放状态，也不能遗漏可能已发送的 down。
        simulated.insert(event.key, 1);
    }
    let results = dispatcher.dispatch(events);
    for (event, result) in events.iter().zip(results.iter().copied()) {
        if event.key.is_wheel() {
            continue;
        }
        if event.is_up && result.was_sent() {
            simulated.remove(&event.key);
        } else if !event.is_up && !result.was_sent() {
            if let Some(count) = previous.get(&event.key) {
                simulated.insert(event.key, *count);
            } else {
                simulated.remove(&event.key);
            }
        }
    }
    results
}

pub(crate) fn release_simulated_keys(keys: &SimulatedKeys) -> bool {
    release_simulated_keys_using(keys, inject)
}

fn release_simulated_keys_using(
    keys: &SimulatedKeys,
    send: impl FnOnce(&[InputEvent]) -> Vec<DispatchResult>,
) -> bool {
    let mut simulated = keys.lock().unwrap_or_else(|e| e.into_inner());
    let events: Vec<_> = simulated.keys().copied().map(InputEvent::up).collect();
    if events.is_empty() {
        return true;
    }
    if let Ok(results) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| send(&events))) {
        for (event, result) in events.iter().zip(results) {
            if result.was_sent() {
                simulated.remove(&event.key);
            }
        }
    }
    if !simulated.is_empty() {
        warn!(
            "仍有 {} 个模拟按键释放失败，保留账本供后续重试",
            simulated.len()
        );
    }
    simulated.is_empty()
}

#[cfg(not(windows))]
mod platform_wait {
    use super::{Duration, Instant, Receiver, SchedulerCommand};
    use std::sync::atomic::AtomicBool;
    use std::sync::mpsc::RecvTimeoutError;
    use std::sync::Arc;

    #[derive(Clone)]
    pub struct CommandWaker;

    impl CommandWaker {
        pub fn new(_degraded: &Arc<AtomicBool>) -> Self {
            Self
        }

        pub fn wake(&self) {}
    }

    pub enum WaitOutcome {
        Command(SchedulerCommand),
        Timeout,
        Disconnected,
    }

    pub struct WaitContext;

    impl WaitContext {
        pub fn new(_command_waker: CommandWaker, _degraded: Arc<AtomicBool>) -> Self {
            Self
        }

        pub fn recv(
            &mut self,
            rx: &Receiver<SchedulerCommand>,
            deadline: Option<Instant>,
        ) -> WaitOutcome {
            match deadline {
                Some(next_at) => {
                    let now = Instant::now();
                    if next_at <= now {
                        return WaitOutcome::Timeout;
                    }
                    recv_timeout(rx, next_at.duration_since(now))
                }
                None => match rx.recv() {
                    Ok(command) => WaitOutcome::Command(command),
                    Err(_) => WaitOutcome::Disconnected,
                },
            }
        }
    }

    fn recv_timeout(rx: &Receiver<SchedulerCommand>, timeout: Duration) -> WaitOutcome {
        match rx.recv_timeout(timeout) {
            Ok(command) => WaitOutcome::Command(command),
            Err(RecvTimeoutError::Timeout) => WaitOutcome::Timeout,
            Err(RecvTimeoutError::Disconnected) => WaitOutcome::Disconnected,
        }
    }
}

#[cfg(windows)]
mod platform_wait {
    use super::{Duration, Instant, Receiver, SchedulerCommand};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc::{RecvTimeoutError, TryRecvError};
    use std::sync::Arc;
    use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
    use windows_sys::Win32::System::Threading::{
        CreateEventW, CreateWaitableTimerExW, SetEvent, SetWaitableTimer, WaitForMultipleObjects,
        CREATE_WAITABLE_TIMER_HIGH_RESOLUTION, INFINITE, TIMER_ALL_ACCESS,
    };

    struct WinHandle(HANDLE);

    unsafe impl Send for WinHandle {}
    unsafe impl Sync for WinHandle {}

    impl Drop for WinHandle {
        fn drop(&mut self) {
            if !self.0.is_null() {
                unsafe { CloseHandle(self.0) };
            }
        }
    }

    #[derive(Clone)]
    pub struct CommandWaker {
        event: Option<Arc<WinHandle>>,
    }

    impl CommandWaker {
        pub fn new(degraded: &Arc<AtomicBool>) -> Self {
            let event = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
            if event.is_null() {
                degraded.store(true, Ordering::SeqCst);
                Self { event: None }
            } else {
                Self {
                    event: Some(Arc::new(WinHandle(event))),
                }
            }
        }

        pub fn wake(&self) {
            if let Some(event) = &self.event {
                unsafe { SetEvent(event.0) };
            }
        }
    }

    pub enum WaitOutcome {
        Command(SchedulerCommand),
        Timeout,
        Disconnected,
    }

    pub struct WaitContext {
        command_waker: CommandWaker,
        timer: Option<WinHandle>,
        degraded: Arc<AtomicBool>,
    }

    impl WaitContext {
        pub fn new(command_waker: CommandWaker, degraded: Arc<AtomicBool>) -> Self {
            let timer = unsafe {
                CreateWaitableTimerExW(
                    std::ptr::null(),
                    std::ptr::null(),
                    CREATE_WAITABLE_TIMER_HIGH_RESOLUTION,
                    TIMER_ALL_ACCESS,
                )
            };
            let timer = if timer.is_null() {
                degraded.store(true, Ordering::SeqCst);
                None
            } else {
                Some(WinHandle(timer))
            };
            Self {
                command_waker,
                timer,
                degraded,
            }
        }

        pub fn recv(
            &mut self,
            rx: &Receiver<SchedulerCommand>,
            deadline: Option<Instant>,
        ) -> WaitOutcome {
            match rx.try_recv() {
                Ok(command) => return WaitOutcome::Command(command),
                Err(TryRecvError::Disconnected) => return WaitOutcome::Disconnected,
                Err(TryRecvError::Empty) => {}
            }

            let Some(event) = self.command_waker.event.as_ref() else {
                return fallback_recv(rx, deadline);
            };
            let Some(timer) = self.timer.as_ref() else {
                return fallback_recv(rx, deadline);
            };

            let handles = match deadline {
                Some(next_at) => {
                    let now = Instant::now();
                    if next_at <= now {
                        return WaitOutcome::Timeout;
                    }
                    let due_time = relative_due_time(next_at.duration_since(now));
                    let ok = unsafe {
                        SetWaitableTimer(timer.0, &due_time, 0, None, std::ptr::null(), 0)
                    };
                    if ok == 0 {
                        self.degraded.store(true, Ordering::SeqCst);
                        return fallback_recv(rx, deadline);
                    }
                    [event.0, timer.0]
                }
                None => [event.0, std::ptr::null_mut()],
            };

            let (count, ptr) = if deadline.is_some() {
                (2, handles.as_ptr())
            } else {
                (1, handles.as_ptr())
            };
            let wait = unsafe { WaitForMultipleObjects(count, ptr, 0, INFINITE) };
            match wait {
                WAIT_OBJECT_0 => match rx.try_recv() {
                    Ok(command) => WaitOutcome::Command(command),
                    Err(TryRecvError::Disconnected) => WaitOutcome::Disconnected,
                    Err(TryRecvError::Empty) => WaitOutcome::Timeout,
                },
                x if x == WAIT_OBJECT_0 + 1 => WaitOutcome::Timeout,
                WAIT_TIMEOUT => WaitOutcome::Timeout,
                _ => {
                    self.degraded.store(true, Ordering::SeqCst);
                    fallback_recv(rx, deadline)
                }
            }
        }
    }

    fn fallback_recv(rx: &Receiver<SchedulerCommand>, deadline: Option<Instant>) -> WaitOutcome {
        match deadline {
            Some(next_at) => {
                let now = Instant::now();
                if next_at <= now {
                    return WaitOutcome::Timeout;
                }
                match rx.recv_timeout(next_at.duration_since(now)) {
                    Ok(command) => WaitOutcome::Command(command),
                    Err(RecvTimeoutError::Timeout) => WaitOutcome::Timeout,
                    Err(RecvTimeoutError::Disconnected) => WaitOutcome::Disconnected,
                }
            }
            None => match rx.recv() {
                Ok(command) => WaitOutcome::Command(command),
                Err(_) => WaitOutcome::Disconnected,
            },
        }
    }

    fn relative_due_time(duration: Duration) -> i64 {
        let ticks = (duration.as_nanos() / 100).clamp(1, i64::MAX as u128) as i64;
        -ticks
    }
}

#[cfg(test)]
mod sim_tests;

#[cfg(test)]
mod safety_tests;
#[cfg(test)]
mod sequence_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use qzh_profile::key_id::MouseButton;
    use qzh_profile::profile::BurstMode;

    struct TestDispatcher;

    impl EventDispatcher for TestDispatcher {
        fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
            vec![DispatchResult::Sent; events.len()]
        }
    }

    /// 记录所有派发事件、并把它们都视为成功（Sent）的派发替身。
    #[derive(Default)]
    struct RecordingDispatcher {
        events: Mutex<Vec<InputEvent>>,
    }

    impl EventDispatcher for RecordingDispatcher {
        fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .extend_from_slice(events);
            vec![DispatchResult::Sent; events.len()]
        }
    }

    impl RecordingDispatcher {
        fn events(&self) -> Vec<InputEvent> {
            self.events
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
        }
    }

    fn test_worker() -> (SchedulerWorker, SimulatedKeys) {
        let simulated_keys = Arc::new(Mutex::new(HashMap::new()));
        let worker = SchedulerWorker {
            rules: HashMap::new(),
            target_holds: HashMap::new(),
            current_generation: 0,
            dispatcher: Arc::new(TestDispatcher),
            stats: None,
            simulated_keys: simulated_keys.clone(),
            min_interval_ms: 1,
            tap_seq: 0,
            progress: Default::default(),
            clock: |now| now,
            cleanup_at: None,
        };
        (worker, simulated_keys)
    }

    fn rule(id: &str, target_key: KeyId) -> Arc<BurstRule> {
        Arc::new(BurstRule {
            id: id.to_string(),
            enabled: true,
            trigger_key: KeyId::Keyboard(0x51),
            steps: vec![qzh_profile::profile::OutputStep {
                key: target_key,
                interval_ms: 10,
            }],
            mode: BurstMode::Toggle,
            stop_key: None,
            group: None,
        })
    }

    #[test]
    fn hold_duration_for_one_ms_uses_tap_mode() {
        assert_eq!(hold_duration(1), Duration::ZERO);
    }

    #[test]
    fn old_generation_start_is_discarded_after_stop_all() {
        let (mut worker, _) = test_worker();
        worker.handle_command(SchedulerCommand::StopAll {
            generation: 1,
            ack: None,
        });
        worker.handle_command(SchedulerCommand::Start {
            rule: rule("old", KeyId::Keyboard(0x45)),
            generation: 0,
            run_id: 1,
            once: false,
        });

        assert!(worker.rules.is_empty());
    }

    #[test]
    fn stale_stop_all_does_not_cleanup_newer_generation_rules() {
        let (mut worker, _) = test_worker();
        worker.current_generation = 2;
        worker.handle_command(SchedulerCommand::Start {
            rule: rule("new", KeyId::Keyboard(0x45)),
            generation: 2,
            run_id: 1,
            once: false,
        });
        worker.process_due(Instant::now() + Duration::from_millis(1));
        let (ack_tx, ack_rx) = mpsc::channel();

        worker.handle_command(SchedulerCommand::StopAll {
            generation: 1,
            ack: Some(ack_tx),
        });

        assert_eq!(ack_rx.try_recv(), Ok(true));
        assert!(worker.rules.contains_key("new"));
        assert_eq!(worker.current_generation, 2);
    }

    #[test]
    fn same_target_owners_share_single_down() {
        let (mut worker, _) = test_worker();
        let key = KeyId::Keyboard(0x45);
        let th = &mut worker.target_holds;
        let a: Arc<str> = Arc::from("a");
        let b: Arc<str> = Arc::from("b");

        assert_eq!(
            SchedulerWorker::acquire_owner(th, key, &a),
            AcquireOutcome::Acquired
        );
        assert_eq!(
            SchedulerWorker::acquire_owner(th, key, &b),
            AcquireOutcome::Shared
        );
        assert_eq!(SchedulerWorker::release_owner(th, key, "a"), None);
        assert_eq!(
            SchedulerWorker::release_owner(th, key, "b"),
            Some(InputEvent::up(key))
        );
    }

    #[test]
    fn acquire_owner_no_longer_gates_on_physical_state() {
        // 引擎不为业务取舍兜底：target 键即便被物理按住也照常注入（由业务层声明风险）。
        let (mut worker, _) = test_worker();
        let key = KeyId::Keyboard(0x45);
        let owner: Arc<str> = Arc::from("different-trigger");

        assert_eq!(
            SchedulerWorker::acquire_owner(&mut worker.target_holds, key, &owner),
            AcquireOutcome::Acquired
        );
        assert!(worker.target_holds.contains_key(&key));
    }

    #[test]
    fn key_up_releases_simulated_down() {
        let (worker, simulated_keys) = test_worker();
        let key = KeyId::Keyboard(0x45);
        simulated_keys.lock().unwrap().insert(key, 1);

        worker.dispatch_events(vec![InputEvent::up(key)]);

        assert!(!simulated_keys.lock().unwrap().contains_key(&key));
    }

    #[test]
    fn cleanup_all_reconciles_residual_ledger_so_no_key_stuck() {
        // 边界（B1）：某次 up 注入失败会让账本残留「按下」、而 target_holds 已清空。
        // cleanup_all 必须按账本补发一次 up，杜绝 OS 端按键永久卡住。
        let dispatcher = Arc::new(RecordingDispatcher::default());
        let simulated_keys = Arc::new(Mutex::new(HashMap::new()));
        let key = KeyId::Keyboard(0x45);
        simulated_keys.lock().unwrap().insert(key, 1); // 模拟失败 up 后的残留按下态
        let mut worker = SchedulerWorker {
            rules: HashMap::new(),
            target_holds: HashMap::new(),
            current_generation: 0,
            dispatcher: dispatcher.clone(),
            stats: None,
            simulated_keys: simulated_keys.clone(),
            min_interval_ms: 1,
            tap_seq: 0,
            progress: Default::default(),
            clock: |now| now,
            cleanup_at: None,
        };

        worker.cleanup_all();

        assert!(dispatcher.events().contains(&InputEvent::up(key)));
        assert!(simulated_keys.lock().unwrap().is_empty());
    }

    #[test]
    fn wheel_down_is_not_tracked_in_simulated_ledger() {
        // 边界（B2）：滚轮瞬发、其 up 走 Noop 永不递减，故 down 也不该计入账本，否则只增不减。
        let (worker, simulated_keys) = test_worker();
        let wheel = KeyId::Mouse(MouseButton::WheelUp);

        worker.dispatch_events(vec![InputEvent::down(wheel)]);

        assert!(simulated_keys.lock().unwrap().is_empty());
    }

    #[test]
    fn tap_once_fires_one_cycle_then_self_removes() {
        // 边界（A1）：滚轮 Hold 走一次性点按——注入一个 down→up 周期后规则自动移除，
        // 不残留、不再排下一拍。
        let (mut worker, _) = test_worker();
        let target = KeyId::Keyboard(0x45);
        worker.handle_command(SchedulerCommand::TapOnce {
            rule: rule("w", target),
            generation: 0,
        });
        assert_eq!(worker.rules.len(), 1);

        // interval_ms=10 → hold>0：首拍发 down 进 UpPhase，第二拍发 up 后自删。
        worker.process_due(Instant::now());
        worker.process_due(Instant::now() + Duration::from_millis(100));

        assert!(worker.rules.is_empty());
        assert!(worker.target_holds.is_empty());
    }

    #[test]
    fn tap_once_with_stale_generation_is_discarded() {
        // 一次性点按同样受 generation 围栏约束：过期代的 tap 不应排程。
        let (mut worker, _) = test_worker();
        worker.current_generation = 2;
        worker.handle_command(SchedulerCommand::TapOnce {
            rule: rule("w", KeyId::Keyboard(0x45)),
            generation: 1,
        });

        assert!(worker.rules.is_empty());
    }
}
