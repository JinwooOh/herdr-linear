use super::*;
use serde_json::json;
use std::time::Duration;

fn issue(id: &str) -> linear::Issue {
    serde_json::from_value(json!({"id":id,"identifier":"HL-1","title":id,"url":"https://linear.app/test","priority":0,"branchName":"test","updatedAt":"2026-01-01T00:00:00Z","team":{"id":"team"},"state":{"id":"s","name":"Todo","type":"started"},"attachments":{"nodes":[]}})).unwrap()
}
fn wait(list: &mut IssueList) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while list.loading() {
        assert!(Instant::now() < deadline, "background result timed out");
        list.poll();
        std::thread::sleep(Duration::from_millis(1));
    }
}
fn fetch_two(key: &str) -> anyhow::Result<Vec<linear::Issue>> {
    assert_eq!(key, "test-key");
    Ok(vec![issue("first"), issue("second")])
}
fn fetch_offline(_: &str) -> anyhow::Result<Vec<linear::Issue>> {
    anyhow::bail!("offline")
}

#[test]
fn refresh_runs_the_fetch_it_is_given_and_applies_its_snapshot() {
    let mut list = IssueList::default();
    list.refresh(&Ok("test-key".into()), fetch_two);
    assert!(list.loading());
    wait(&mut list);
    let ids: Vec<_> = list.issues.iter().map(|i| i.id.as_str()).collect();
    // Same state and updatedAt: ties sort by issue ID.
    assert_eq!(ids, ["first", "second"]);
    assert_eq!(list.selected().map(|i| i.id.as_str()), Some("first"));
    assert!(list.error.is_none());
}

#[test]
fn a_failed_fetch_keeps_the_last_issues_and_records_the_error() {
    let mut list = IssueList::default();
    list.apply_snapshot(vec![issue("kept")]);
    list.refresh(&Ok("test-key".into()), fetch_offline);
    wait(&mut list);
    assert_eq!(list.issues.len(), 1);
    assert_eq!(list.error.as_deref(), Some("offline"));
}

#[test]
fn a_missing_key_records_the_error_without_starting_a_fetch() {
    let mut list = IssueList::default();
    list.refresh(&Err("work key unavailable".into()), fetch_two);
    assert!(!list.loading());
    assert_eq!(list.error.as_deref(), Some("work key unavailable"));
}
