//! 引擎级测试共用件：命令录制调度器替身与规则构造器。

use crate::scheduler::Scheduler;
use crate::BurstEngine;
use qzh_profile::key_id::KeyId;
use qzh_profile::profile::{BurstMode, BurstRule};
use std::sync::{Arc, Mutex};

/// 只记录引擎下发命令、不做任何调度的替身。阻塞类命令一律返回 true（成功），
/// 使引擎走正常路径而非 simulated_keys 兜底。
#[derive(Default)]
pub(crate) struct RecordingScheduler {
    cmds: Mutex<Vec<String>>,
}

impl RecordingScheduler {
    pub(crate) fn clear(&self) {
        self.cmds.lock().unwrap().clear();
    }
    pub(crate) fn cmds(&self) -> Vec<String> {
        self.cmds.lock().unwrap().clone()
    }
    /// 取走已录制的命令，便于分段断言。
    pub(crate) fn take(&self) -> Vec<String> {
        std::mem::take(&mut *self.cmds.lock().unwrap())
    }
    fn log(&self, s: String) {
        self.cmds.lock().unwrap().push(s);
    }
}

impl Scheduler for RecordingScheduler {
    fn resume_rule(&self, rule_id: String, generation: u64) {
        self.log(format!("resume:{rule_id}:g{generation}"));
    }
    fn play_once(&self, rule: Arc<BurstRule>, generation: u64) {
        self.log(format!("preview:{}:g{generation}", rule.id));
    }
    fn pause_rule(&self, rule_id: String, generation: u64) {
        self.log(format!("pause:{rule_id}:g{generation}"));
    }
    fn start_rule(&self, rule: Arc<BurstRule>, generation: u64) {
        self.log(format!("start:{}:g{generation}", rule.id));
    }
    fn tap_once(&self, rule: Arc<BurstRule>, generation: u64) {
        self.log(format!("tap:{}:g{generation}", rule.id));
    }
    fn stop_rule(&self, rule_id: String, generation: u64) {
        self.log(format!("stop:{rule_id}:g{generation}"));
    }
    fn stop_all_async(&self, generation: u64) {
        self.log(format!("stopall:g{generation}"));
    }
    fn stop_all_blocking(&self, generation: u64) -> bool {
        self.log(format!("stopall_blocking:g{generation}"));
        true
    }
    fn shutdown_blocking(&self, generation: u64) -> bool {
        self.log(format!("shutdown:g{generation}"));
        true
    }
    fn hp_degraded(&self) -> bool {
        false
    }
}

pub(crate) fn rule(id: &str, mode: BurstMode, trigger: KeyId, target: KeyId) -> BurstRule {
    BurstRule {
        id: id.to_string(),
        enabled: true,
        trigger_key: trigger,
        steps: vec![qzh_profile::profile::OutputStep {
            key: target,
            interval_ms: 10,
        }],
        mode,
        stop_key: None,
        group: None,
    }
}

/// 建好引擎、装规则、开全局开关，并清掉装规则引发的 stop_all，
/// 使后续断言只看「按键引发的命令」。装规则会把 generation 推进到 1。
pub(crate) fn setup(rules: Vec<BurstRule>) -> (BurstEngine, Arc<RecordingScheduler>) {
    let rec = Arc::new(RecordingScheduler::default());
    let engine = BurstEngine::new_with_scheduler(rec.clone());
    engine.set_rules(rules);
    engine.set_global_enabled(true, false);
    rec.clear();
    (engine, rec)
}
