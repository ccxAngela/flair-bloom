use chrono::{DateTime, SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet, VecDeque};
use uuid::Uuid;

pub const QUEUE_ITEMS: usize = 1_000;
pub const QUEUE_BYTES: usize = 1_048_576;
pub const BATCH_ITEMS: usize = 50;
pub const BATCH_BYTES: usize = 65_536;
pub const OFFLINE_MS: i64 = 7 * 86_400_000;
pub const FLUSH_MS: i64 = 30_000;

pub fn id() -> String {
    Uuid::new_v4().to_string()
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Environment {
    pub id: String,
    pub source: String,
    pub version: String,
    pub scope: String,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Identity {
    pub state: String,
    pub segment_id: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum EventName {
    SessionStarted,
    RuntimeReady,
    RuntimeFailed,
    RuleActivity,
    InputModeResult,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Event {
    pub schema_version: u8,
    pub event_id: String,
    pub session_id: String,
    pub build: String,
    pub occurred_at: String,
    pub environment: Environment,
    pub identity: Identity,
    pub name: EventName,
    pub fields: BTreeMap<String, String>,
}

impl Event {
    fn time(&self) -> Option<i64> {
        DateTime::parse_from_rfc3339(&self.occurred_at)
            .ok()
            .map(|time| time.timestamp_millis())
    }

    fn valid(&self) -> bool {
        let text = |value: &str, limit| {
            !value.is_empty()
                && value.chars().count() <= limit
                && !value.chars().any(char::is_control)
        };
        self.schema_version == 1
            && self.time().is_some()
            && text(&self.event_id, 128)
            && text(&self.session_id, 128)
            && text(&self.build, 128)
            && text(&self.environment.id, 256)
            && text(&self.environment.source, 64)
            && text(&self.environment.version, 64)
            && ["persistent", "session"].contains(&self.environment.scope.as_str())
            && self.identity.state == "anonymous"
            && text(&self.identity.segment_id, 128)
            && match self.name {
                EventName::RuntimeFailed => {
                    self.fields.len() == 1
                        && self.fields.get("reason").is_some_and(|v| {
                            ["unsupported", "initialization", "unavailable", "unknown"]
                                .contains(&v.as_str())
                        })
                }
                EventName::InputModeResult => {
                    self.fields.len() == 3
                        && self.fields.get("mode").is_some_and(|v| {
                            ["sendinput", "interception", "ddsimple", "hidstick"]
                                .contains(&v.as_str())
                        })
                        && self
                            .fields
                            .get("result")
                            .is_some_and(|v| ["success", "failed"].contains(&v.as_str()))
                        && self.fields.get("reason").is_some_and(|v| {
                            [
                                "none",
                                "unsupported",
                                "permission",
                                "driver",
                                "unavailable",
                                "unknown",
                            ]
                            .contains(&v.as_str())
                        })
                }
                _ => self.fields.is_empty(),
            }
    }
}

#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueueFile {
    pub items: VecDeque<Event>,
}

#[derive(Debug, Deserialize)]
pub struct Receipt {
    pub items: Vec<ReceiptItem>,
}
#[derive(Debug, Deserialize)]
pub struct ReceiptItem {
    pub index: usize,
    pub status: ReceiptStatus,
}
#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReceiptStatus {
    Accepted,
    Duplicate,
    Rejected,
    Retry,
}

pub struct Collector {
    pub queue: QueueFile,
    pub enabled: bool,
    pub environment: Option<Environment>,
    pub next_flush: i64,
    session: String,
    segment: String,
    activity_days: HashSet<i64>,
    attempts: u32,
}

impl Collector {
    pub fn new(
        enabled: bool,
        environment: Option<Environment>,
        queue: QueueFile,
        now: i64,
    ) -> Self {
        let mut collector = Self {
            queue,
            enabled,
            environment,
            next_flush: now + FLUSH_MS,
            session: id(),
            segment: id(),
            activity_days: HashSet::new(),
            attempts: 0,
        };
        if !enabled {
            collector.queue.items.clear();
        }
        collector.prune(now);
        collector
    }

    pub fn toggle(&mut self, enabled: bool, environment: Option<Environment>, now: i64) {
        self.enabled = enabled;
        self.environment = if enabled { environment } else { None };
        self.queue.items.clear();
        self.activity_days.clear();
        self.segment = id();
        self.attempts = 0;
        self.next_flush = now + FLUSH_MS;
        if enabled {
            self.record(EventName::SessionStarted, BTreeMap::new(), now);
        }
    }

    pub fn record(&mut self, name: EventName, fields: BTreeMap<String, String>, now: i64) {
        if !self.enabled {
            return;
        }
        let Some(environment) = self.environment.clone() else {
            return;
        };
        let Some(time) = DateTime::<Utc>::from_timestamp_millis(now) else {
            return;
        };
        let event = Event {
            schema_version: 1,
            event_id: id(),
            session_id: self.session.clone(),
            build: env!("CARGO_PKG_VERSION").into(),
            occurred_at: time.to_rfc3339_opts(SecondsFormat::Millis, true),
            environment,
            identity: Identity {
                state: "anonymous".into(),
                segment_id: self.segment.clone(),
            },
            name,
            fields,
        };
        if event.valid() {
            if name == EventName::RuleActivity {
                let day = (now + 8 * 3_600_000).div_euclid(86_400_000);
                if !self.activity_days.insert(day) {
                    return;
                }
                self.activity_days
                    .retain(|day| *day >= (now - OFFLINE_MS).div_euclid(86_400_000));
            }
            self.queue.items.push_back(event);
        }
        self.prune(now);
    }

    pub fn prune(&mut self, now: i64) {
        self.queue.items.retain(|event| {
            event.valid() && event.time().is_some_and(|time| time >= now - OFFLINE_MS)
        });
        while self.queue.items.len() > QUEUE_ITEMS {
            self.queue.items.pop_front();
        }
        while serde_json::to_vec(&self.queue).map_or(usize::MAX, |bytes| bytes.len()) > QUEUE_BYTES
            && !self.queue.items.is_empty()
        {
            self.queue.items.pop_front();
        }
    }

    pub fn batch(&mut self, now: i64) -> Vec<Event> {
        self.prune(now);
        if !self.enabled {
            return Vec::new();
        }
        let mut batch = Vec::new();
        for event in self.queue.items.iter().take(BATCH_ITEMS) {
            batch.push(event.clone());
            if serde_json::to_vec(&serde_json::json!({ "items": batch }))
                .map_or(usize::MAX, |v| v.len())
                > BATCH_BYTES
            {
                batch.pop();
                break;
            }
        }
        batch
    }

    pub fn acknowledge(&mut self, batch: &[Event], receipt: Receipt, now: i64, jitter: u64) {
        // 下标重复或越界时整份回执不可信，保留事件交给服务端幂等处理。
        let mut seen = HashSet::new();
        if receipt
            .items
            .iter()
            .any(|item| item.index >= batch.len() || !seen.insert(item.index))
        {
            self.retry(now, jitter);
            return;
        }
        let mut removed = HashSet::new();
        for item in receipt.items {
            if !matches!(item.status, ReceiptStatus::Retry) {
                removed.insert(batch[item.index].event_id.as_str());
            }
        }
        self.queue
            .items
            .retain(|event| !removed.contains(event.event_id.as_str()));
        if removed.len() == batch.len() {
            self.attempts = 0;
            self.next_flush = now + FLUSH_MS;
        } else {
            self.retry(now, jitter);
        }
    }

    pub fn retry(&mut self, now: i64, jitter: u64) {
        let base = (2_000_u64.saturating_mul(1_u64 << self.attempts.min(8))).min(300_000);
        self.attempts = self.attempts.saturating_add(1);
        self.next_flush = now + (base / 2 + jitter % (base / 2 + 1)) as i64;
    }

    pub fn reject_batch(&mut self, batch: &[Event], now: i64) {
        let ids: HashSet<_> = batch.iter().map(|event| event.event_id.as_str()).collect();
        self.queue
            .items
            .retain(|event| !ids.contains(event.event_id.as_str()));
        self.attempts = 0;
        self.next_flush = now + FLUSH_MS;
    }
}

#[cfg(test)]
mod tests;
