use super::*;

const NOW: i64 = 1_794_000_000_000;
fn environment() -> Environment {
    Environment {
        id: "test-environment".into(),
        source: "random".into(),
        version: "1".into(),
        scope: "persistent".into(),
    }
}
fn collector() -> Collector {
    Collector::new(true, Some(environment()), QueueFile::default(), NOW)
}

#[test]
fn usage_checkpoints_coalesce_per_session_day_without_erasing_start_or_layout() {
    let mut core = collector();
    core.record(EventName::SessionStarted, BTreeMap::new(), NOW);
    core.record(
        EventName::LayoutObserved,
        BTreeMap::from([("layout".into(), "horizontal".into())]),
        NOW,
    );
    for duration in [1000, 2000, 3000] {
        core.record(
            EventName::SessionUsage,
            BTreeMap::from([("durationMs".into(), duration.to_string())]),
            NOW + duration,
        );
    }
    assert_eq!(core.queue.items.len(), 3);
    assert_eq!(
        core.queue.items.back().unwrap().fields["durationMs"],
        "3000"
    );
    for value in ["-1", "01", "+1", "86400001", "1.5"] {
        core.record(
            EventName::SessionUsage,
            BTreeMap::from([("durationMs".into(), value.into())]),
            NOW,
        );
    }
    assert_eq!(core.queue.items.len(), 3);
    core.record(
        EventName::SessionUsage,
        BTreeMap::from([("durationMs".into(), "500".into())]),
        NOW + 86_400_000,
    );
    assert_eq!(core.queue.items.len(), 4);
}
fn receipt(statuses: &[ReceiptStatus]) -> Receipt {
    Receipt {
        items: statuses
            .iter()
            .enumerate()
            .map(|(index, status)| ReceiptItem {
                index,
                status: *status,
            })
            .collect(),
    }
}

#[test]
fn disabled_drops_persisted_queue_and_does_not_record() {
    let mut core = collector();
    core.record(EventName::SessionStarted, BTreeMap::new(), NOW);
    let mut disabled = Collector::new(false, None, core.queue, NOW);
    disabled.record(EventName::RuleActivity, BTreeMap::new(), NOW);
    assert!(disabled.queue.items.is_empty());
    assert!(disabled.environment.is_none());
}

#[test]
fn enabling_observes_current_time_and_keeps_process_and_environment() {
    let mut core = collector();
    core.record(EventName::RuleActivity, BTreeMap::new(), NOW);
    let original = core.queue.items[0].clone();
    core.toggle(false, None, NOW);
    core.record(EventName::RuleActivity, BTreeMap::new(), NOW + 1);
    core.toggle(true, Some(environment()), NOW + 2);
    core.record(EventName::RuleActivity, BTreeMap::new(), NOW + 3);
    assert_eq!(core.queue.items.len(), 2);
    let observed = &core.queue.items[1];
    assert_eq!(observed.session_id, original.session_id);
    assert_eq!(observed.environment, original.environment);
    assert_ne!(observed.identity.segment_id, original.identity.segment_id);
    assert_eq!(core.queue.items[0].time(), Some(NOW + 2));
}

#[test]
fn activities_are_aggregated_by_observed_segment_and_shanghai_day() {
    let mut core = collector();
    let midnight = (NOW + 8 * 3_600_000).div_euclid(86_400_000) * 86_400_000 - 8 * 3_600_000;
    for now in [midnight - 1, midnight - 1, midnight, midnight + 1] {
        core.record(EventName::RuleActivity, BTreeMap::new(), now);
    }
    assert_eq!(core.queue.items.len(), 2);
}

#[test]
fn queue_limits_items_bytes_and_age_and_survives_restart() {
    let mut core = collector();
    for i in 0..1_020 {
        core.record(EventName::SessionStarted, BTreeMap::new(), NOW + i);
    }
    assert_eq!(core.queue.items.len(), QUEUE_ITEMS);
    let file = serde_json::to_vec(&core.queue).unwrap();
    assert!(file.len() <= QUEUE_BYTES);
    let mut restored = Collector::new(
        true,
        Some(environment()),
        serde_json::from_slice(&file).unwrap(),
        NOW + 1_020,
    );
    assert_eq!(restored.queue.items.len(), QUEUE_ITEMS);
    let old_id = restored.queue.items[0].event_id.clone();
    assert_eq!(restored.batch(NOW + 1_020)[0].event_id, old_id);
    restored.prune(NOW + OFFLINE_MS + 1_021);
    assert!(restored.queue.items.is_empty());
}

#[test]
fn queue_limits_bytes_independently_of_item_count() {
    let mut core = collector();
    for _ in 0..1_000 {
        core.record(EventName::SessionStarted, BTreeMap::new(), NOW);
        let event = core.queue.items.back_mut().unwrap();
        event.environment.id = "界".repeat(256);
        event.environment.source = "界".repeat(64);
        event.environment.version = "界".repeat(64);
        event.build = "界".repeat(128);
    }
    core.prune(NOW);
    assert!(core.queue.items.len() < QUEUE_ITEMS);
    assert!(serde_json::to_vec(&core.queue).unwrap().len() <= QUEUE_BYTES);
    let batch = core.batch(NOW);
    assert!(!batch.is_empty());
    assert!(
        serde_json::to_vec(&serde_json::json!({"items": batch}))
            .unwrap()
            .len()
            <= BATCH_BYTES
    );
}

#[test]
fn partial_receipt_removes_permanent_errors_and_preserves_retry_and_missing() {
    let mut core = collector();
    for _ in 0..5 {
        core.record(EventName::SessionStarted, BTreeMap::new(), NOW);
    }
    let batch = core.batch(NOW);
    core.acknowledge(
        &batch,
        receipt(&[
            ReceiptStatus::Accepted,
            ReceiptStatus::Duplicate,
            ReceiptStatus::Rejected,
            ReceiptStatus::Retry,
        ]),
        NOW,
        7,
    );
    assert_eq!(
        core.queue
            .items
            .iter()
            .map(|event| &event.event_id)
            .collect::<Vec<_>>(),
        vec![&batch[3].event_id, &batch[4].event_id]
    );
    assert!(core.next_flush > NOW && core.next_flush <= NOW + 2_000);
}

#[test]
fn malformed_receipt_never_drops_events() {
    for indices in [[0, 0], [0, 2]] {
        let mut core = collector();
        for _ in 0..2 {
            core.record(EventName::SessionStarted, BTreeMap::new(), NOW);
        }
        let batch = core.batch(NOW);
        core.acknowledge(
            &batch,
            Receipt {
                items: indices
                    .into_iter()
                    .map(|index| ReceiptItem {
                        index,
                        status: ReceiptStatus::Accepted,
                    })
                    .collect(),
            },
            NOW,
            1,
        );
        assert_eq!(core.queue.items.len(), 2);
    }
}

#[test]
fn backoff_is_bounded_and_recovers_after_success() {
    let mut core = collector();
    for attempt in 0..20 {
        core.retry(NOW, attempt);
        assert!(core.next_flush > NOW && core.next_flush <= NOW + 300_000);
    }
    core.record(EventName::SessionStarted, BTreeMap::new(), NOW);
    let batch = core.batch(NOW);
    core.acknowledge(&batch, receipt(&[ReceiptStatus::Accepted]), NOW, 0);
    assert_eq!(core.next_flush, NOW + FLUSH_MS);
    assert!(core.queue.items.is_empty());
}

#[test]
fn permanent_bad_batch_does_not_block_the_next_fifty_events() {
    let mut core = collector();
    for _ in 0..60 {
        core.record(EventName::SessionStarted, BTreeMap::new(), NOW);
    }
    let batch = core.batch(NOW);
    assert_eq!(batch.len(), BATCH_ITEMS);
    let remaining = core.queue.items[BATCH_ITEMS].event_id.clone();
    core.reject_batch(&batch, NOW);
    assert_eq!(core.queue.items.len(), 10);
    assert_eq!(core.batch(NOW)[0].event_id, remaining);
}

#[test]
fn disabled_queue_cannot_be_restored_by_late_receipts() {
    let mut core = collector();
    core.record(EventName::RuleActivity, BTreeMap::new(), NOW);
    let batch = core.batch(NOW);
    core.toggle(false, None, NOW);
    core.acknowledge(&batch, receipt(&[ReceiptStatus::Retry]), NOW, 0);
    assert!(core.queue.items.is_empty());
    assert!(core.batch(NOW).is_empty());
}

#[test]
fn only_fixed_fields_and_anonymous_identity_enter_queue() {
    let mut core = collector();
    core.record(
        EventName::RuleActivity,
        BTreeMap::from([("key".into(), "secret".into())]),
        NOW,
    );
    core.record(
        EventName::InputModeResult,
        BTreeMap::from([
            ("mode".into(), "sendinput".into()),
            ("result".into(), "success".into()),
            ("reason".into(), "none".into()),
        ]),
        NOW,
    );
    assert_eq!(core.queue.items.len(), 1);
    assert_eq!(core.queue.items[0].identity.state, "anonymous");
    let json = serde_json::to_value(&core.queue.items[0]).unwrap();
    assert_eq!(json["schemaVersion"], 1);
    assert!(json["occurredAt"].as_str().unwrap().ends_with(".000Z"));
    assert!(json["identity"].get("bindingId").is_none());
}

#[test]
#[ignore = "设置 FLAIR_INSIGHTS_REPLAY 后，单独读取 Platform 所有者仓的公共回放"]
fn shared_platform_replay() {
    let path = std::env::var("FLAIR_INSIGHTS_REPLAY").expect("需要公共 replay.json 路径");
    let replay: serde_json::Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let base = replay["baseEvent"].clone();
    let event: Event = serde_json::from_value(base.clone()).unwrap();
    assert!(event.valid());
    let scenarios = replay["scenarios"].as_array().unwrap();
    let s9 = scenarios
        .iter()
        .find(|scenario| scenario["id"] == "S9")
        .unwrap();
    for (patch, expected) in s9["mutations"]
        .as_array()
        .unwrap()
        .iter()
        .zip(s9["expected"].as_array().unwrap())
    {
        let mut value = base.clone();
        value
            .as_object_mut()
            .unwrap()
            .extend(patch.as_object().unwrap().clone());
        let actual = match serde_json::from_value::<Event>(value) {
            Ok(event) if event.schema_version != 1 => "unsupported_schema",
            Ok(event) if event.valid() => "accepted",
            _ => "invalid_event",
        };
        assert_eq!(actual, expected.as_str().unwrap());
    }
    let s8 = scenarios
        .iter()
        .find(|scenario| scenario["id"] == "S8")
        .unwrap();
    let mut core = collector();
    core.toggle(false, None, NOW);
    let restored = Collector::new(false, None, core.queue, NOW);
    assert_eq!(
        restored.enabled,
        s8["expected"]["enabledAfterRestart"].as_bool().unwrap()
    );
    assert_eq!(
        restored.queue.items.len() as u64,
        s8["expected"]["queuedAfterDisable"].as_u64().unwrap()
    );
}
