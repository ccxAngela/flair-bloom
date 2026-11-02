use crate::scheduler::{EventDispatcher, SchedulerHandle, SchedulerStats};
use qzh_profile::key_id::KeyId;
use qzh_profile::profile::{BurstMode, BurstRule};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};
use win_input::{DispatchResult, InputEvent};

#[derive(Debug, Clone)]
pub struct StressConfig {
    pub rules: usize,
    pub interval_ms: u32,
    pub duration: Duration,
    pub same_target: bool,
    /// 每事件「模拟注入耗时」：>0 时 dispatcher 按此 sleep，复现下游背压（LL hook 链 /
    /// 前台输入队列拥塞），用于在真线程 + 真定时器上观测背压对注入速率/调度迟到的影响
    /// （`dispatch_cost_*` 仅作诊断采样，不驱动任何降频）。
    pub simulated_dispatch_cost: Duration,
    /// 背压起始延迟：注入开始多久后才施加耗时。模拟「积压逐渐建立」的 ramp，使诊断信号
    /// 先建立健康基线再观测骤增。默认 0（从头拥塞，会被当作基线）。
    pub simulated_dispatch_cost_delay: Duration,
}

impl Default for StressConfig {
    fn default() -> Self {
        Self {
            rules: 64,
            interval_ms: 10,
            duration: Duration::from_secs(5),
            same_target: false,
            simulated_dispatch_cost: Duration::ZERO,
            simulated_dispatch_cost_delay: Duration::ZERO,
        }
    }
}

#[derive(Debug)]
pub struct StressReport {
    pub rules: usize,
    pub interval_ms: u32,
    pub duration_ms: u128,
    pub same_target: bool,
    pub scheduler_threads: usize,
    pub hp_degraded: bool,
    pub dispatch_batches: u64,
    pub sent_events: u64,
    pub failed_events: u64,
    pub injection_rate_per_sec: f64,
    pub delay_samples: usize,
    pub delay_p50_us: u128,
    pub delay_p95_us: u128,
    pub delay_p99_us: u128,
    pub delay_max_us: u128,
    pub dispatch_cost_p50_us: u128,
    pub dispatch_cost_p99_us: u128,
    pub dispatch_cost_max_us: u128,
    pub stop_ack_us: u128,
    pub process_cpu_ms: Option<u128>,
}

struct DryRunDispatcher {
    cost_per_event: Duration,
    /// 背压起始延迟：注入开始 `cost_start_after` 之后才施加耗时。模拟真实场景里
    /// 「队列起初为空、注入很快，积压逐渐建立才变慢」的 ramp，让诊断信号先建立健康
    /// 基线、再观测到骤增（否则 t0 即拥塞会被当成基线，看不出相对变化）。
    cost_start_after: Duration,
    started_at: Mutex<Option<Instant>>,
}

impl EventDispatcher for DryRunDispatcher {
    fn dispatch(&self, events: &[InputEvent]) -> Vec<DispatchResult> {
        // 模拟下游背压：注入是同步调用，拥塞时变慢。用真 sleep 让真线程 + 真定时器上的
        // 诊断采样（dispatch_cost / delay）拿到真实信号（非单测，可接受 wall-clock）。
        if !self.cost_per_event.is_zero() {
            let start = *self
                .started_at
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .get_or_insert_with(Instant::now);
            if start.elapsed() >= self.cost_start_after {
                thread::sleep(self.cost_per_event * events.len() as u32);
            }
        }
        vec![DispatchResult::Sent; events.len()]
    }
}

pub fn run_stress(config: StressConfig) -> StressReport {
    let stats = Arc::new(SchedulerStats::default());
    let scheduler = SchedulerHandle::start_with_dispatcher(
        Arc::new(DryRunDispatcher {
            cost_per_event: config.simulated_dispatch_cost,
            cost_start_after: config.simulated_dispatch_cost_delay,
            started_at: Mutex::new(None),
        }),
        Some(stats.clone()),
        Arc::new(Mutex::new(HashMap::new())),
    );
    let cpu_start = process_cpu_time_100ns();
    let started = Instant::now();
    let rules = make_rules(&config);

    for rule in rules {
        scheduler.start_rule(rule, 0);
    }

    thread::sleep(config.duration);

    let stop_started = Instant::now();
    let _ = scheduler.stop_all_blocking(1);
    let stop_ack = stop_started.elapsed();
    let elapsed = started.elapsed();
    let cpu_end = process_cpu_time_100ns();
    let hp_degraded = scheduler.hp_degraded();
    let _ = scheduler.shutdown_blocking(2);
    let snapshot = stats.snapshot();
    let delays = sorted(snapshot.delay_ns);
    let dispatch_costs = sorted(snapshot.dispatch_cost_ns);

    StressReport {
        rules: config.rules,
        interval_ms: config.interval_ms,
        duration_ms: elapsed.as_millis(),
        same_target: config.same_target,
        scheduler_threads: 1,
        hp_degraded,
        dispatch_batches: snapshot.batches,
        sent_events: snapshot.sent_events,
        failed_events: snapshot.failed_events,
        injection_rate_per_sec: snapshot.sent_events as f64 / elapsed.as_secs_f64().max(0.001),
        delay_samples: delays.len(),
        delay_p50_us: percentile_us(&delays, 50),
        delay_p95_us: percentile_us(&delays, 95),
        delay_p99_us: percentile_us(&delays, 99),
        delay_max_us: delays.last().copied().unwrap_or(0) / 1_000,
        dispatch_cost_p50_us: percentile_us(&dispatch_costs, 50),
        dispatch_cost_p99_us: percentile_us(&dispatch_costs, 99),
        dispatch_cost_max_us: dispatch_costs.last().copied().unwrap_or(0) / 1_000,
        stop_ack_us: stop_ack.as_micros(),
        process_cpu_ms: cpu_start
            .zip(cpu_end)
            .map(|(start, end)| end.saturating_sub(start) as u128 / 10_000),
    }
}

impl StressReport {
    pub fn to_json_line(&self) -> String {
        format!(
            "{{\"rules\":{},\"interval_ms\":{},\"duration_ms\":{},\"same_target\":{},\"scheduler_threads\":{},\"hp_degraded\":{},\"dispatch_batches\":{},\"sent_events\":{},\"failed_events\":{},\"injection_rate_per_sec\":{:.2},\"delay_samples\":{},\"delay_p50_us\":{},\"delay_p95_us\":{},\"delay_p99_us\":{},\"delay_max_us\":{},\"dispatch_cost_p50_us\":{},\"dispatch_cost_p99_us\":{},\"dispatch_cost_max_us\":{},\"stop_ack_us\":{},\"process_cpu_ms\":{}}}",
            self.rules,
            self.interval_ms,
            self.duration_ms,
            self.same_target,
            self.scheduler_threads,
            self.hp_degraded,
            self.dispatch_batches,
            self.sent_events,
            self.failed_events,
            self.injection_rate_per_sec,
            self.delay_samples,
            self.delay_p50_us,
            self.delay_p95_us,
            self.delay_p99_us,
            self.delay_max_us,
            self.dispatch_cost_p50_us,
            self.dispatch_cost_p99_us,
            self.dispatch_cost_max_us,
            self.stop_ack_us,
            self.process_cpu_ms
                .map(|v| v.to_string())
                .unwrap_or_else(|| "null".to_string()),
        )
    }
}

fn make_rules(config: &StressConfig) -> Vec<Arc<BurstRule>> {
    (0..config.rules)
        .map(|i| {
            Arc::new(BurstRule {
                id: format!("stress-{i}"),
                enabled: true,
                trigger_key: KeyId::Keyboard(0x1000 + i as u32),
                steps: vec![qzh_profile::profile::OutputStep {
                    key: if config.same_target {
                        KeyId::Keyboard(0x2000)
                    } else {
                        KeyId::Keyboard(0x2000 + i as u32)
                    },
                    interval_ms: config.interval_ms,
                }],
                mode: BurstMode::Toggle,
                stop_key: None,
                group: None,
            })
        })
        .collect()
}

fn sorted(mut values: Vec<u128>) -> Vec<u128> {
    values.sort_unstable();
    values
}

fn percentile_us(values: &[u128], percentile: usize) -> u128 {
    if values.is_empty() {
        return 0;
    }
    let idx = ((values.len() - 1) * percentile) / 100;
    values[idx] / 1_000
}

#[cfg(windows)]
fn process_cpu_time_100ns() -> Option<u64> {
    use windows_sys::Win32::Foundation::FILETIME;
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, GetProcessTimes};

    unsafe {
        let mut creation = std::mem::zeroed::<FILETIME>();
        let mut exit = std::mem::zeroed::<FILETIME>();
        let mut kernel = std::mem::zeroed::<FILETIME>();
        let mut user = std::mem::zeroed::<FILETIME>();
        if GetProcessTimes(
            GetCurrentProcess(),
            &mut creation,
            &mut exit,
            &mut kernel,
            &mut user,
        ) == 0
        {
            return None;
        }
        Some(filetime_to_u64(kernel).saturating_add(filetime_to_u64(user)))
    }
}

#[cfg(windows)]
fn filetime_to_u64(ft: windows_sys::Win32::Foundation::FILETIME) -> u64 {
    ((ft.dwHighDateTime as u64) << 32) | ft.dwLowDateTime as u64
}

#[cfg(not(windows))]
fn process_cpu_time_100ns() -> Option<u64> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stress_report_emits_json_line_with_core_fields() {
        let report = StressReport {
            rules: 1,
            interval_ms: 10,
            duration_ms: 100,
            same_target: false,
            scheduler_threads: 1,
            hp_degraded: false,
            dispatch_batches: 2,
            sent_events: 4,
            failed_events: 0,
            injection_rate_per_sec: 40.0,
            delay_samples: 2,
            delay_p50_us: 1,
            delay_p95_us: 2,
            delay_p99_us: 2,
            delay_max_us: 2,
            dispatch_cost_p50_us: 30,
            dispatch_cost_p99_us: 80,
            dispatch_cost_max_us: 120,
            stop_ack_us: 100,
            process_cpu_ms: Some(5),
        };

        let line = report.to_json_line();
        assert!(line.contains("\"rules\":1"));
        assert!(line.contains("\"process_cpu_ms\":5"));
        assert!(line.contains("\"dispatch_cost_p99_us\":80"));
    }
}
