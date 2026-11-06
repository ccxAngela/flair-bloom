use serde_json::json;

use super::*;

fn valid(tag: &str) -> Value {
    json!({
        "schema": SCHEMA,
        "updated_at": "2026-10-29",
        "schools": [],
        "whitepapers": [{ "url": format!("https://example.com/{tag}") }],
        "macros": [{ "url": "https://www.jx3box.com/macro/1" }],
    })
}

#[test]
fn bundled_snapshot_is_accepted() {
    assert_eq!(verdict(&bundled()), Verdict::Ok);
}

#[test]
fn remote_wins_when_valid() {
    let r = choose(Some(valid("remote")), Some((1, valid("cache"))), 100);
    assert_eq!(r.source, ColumnSource::Remote);
    assert_eq!(r.fetched_at, Some(100));
    assert_eq!(r.fallback, None);
}

#[test]
fn offline_falls_back_to_cache_then_bundled() {
    let r = choose(None, Some((42, valid("cache"))), 100);
    assert_eq!(r.source, ColumnSource::Cache);
    assert_eq!(r.fetched_at, Some(42));
    assert_eq!(r.fallback, Some(Fallback::Offline));

    let r = choose(None, None, 100);
    assert_eq!(r.source, ColumnSource::Bundled);
    assert_eq!(r.fetched_at, None);
}

#[test]
fn newer_schema_keeps_old_data_and_says_why() {
    let mut newer = valid("remote");
    newer["schema"] = json!(SCHEMA + 1);
    let r = choose(Some(newer), Some((42, valid("cache"))), 100);
    assert_eq!(r.source, ColumnSource::Cache);
    assert_eq!(r.fallback, Some(Fallback::NewerSchema));
}

#[test]
fn malformed_remote_or_cache_is_skipped() {
    let r = choose(
        Some(json!({ "schema": SCHEMA })),
        Some((1, json!("坏的"))),
        100,
    );
    assert_eq!(r.source, ColumnSource::Bundled);
    assert_eq!(r.fallback, Some(Fallback::Invalid));
}

#[test]
fn only_https_links_present_in_data_are_allowed() {
    let data = valid("a");
    assert!(link_allowed(&data, "https://example.com/a"));
    assert!(link_allowed(&data, "https://www.jx3box.com/macro/1"));
    assert!(!link_allowed(&data, "https://example.com/other"));
    assert!(!link_allowed(
        &json!({ "whitepapers": [{ "url": "http://x.com/a" }] }),
        "http://x.com/a"
    ));
    assert!(!link_allowed(&json!({}), "https://example.com/a"));
}

#[test]
fn xwink_links_share_the_application_domain_policy() {
    let data = json!({});
    assert!(link_allowed(&data, "https://xwink.fun/"));
    assert!(link_allowed(&data, "https://app.xwink.fun/flair-bloom/"));
    assert!(link_allowed(&data, "https://one.two.xwink.fun/page"));
    assert!(!link_allowed(&data, "https://xwink.fun.example.com/"));
    assert!(!link_allowed(&data, "http://app.xwink.fun/"));
}

/// 打真实的产品站：站点发布后跑 `cargo test -p flair-bloom live_remote -- --ignored`。
#[test]
#[ignore = "联网，站点发布后手动跑"]
fn live_remote_column_is_accepted() {
    let data = tauri::async_runtime::block_on(fetch_remote()).expect("取不到线上数据");
    assert_eq!(verdict(&data), Verdict::Ok);
}
