use super::*;

fn issue(id: &str, updated: &str) -> Issue {
    serde_json::from_value(json!({"id":id,"identifier":"HL-1","title":"test","url":"https://linear.app/test","priority":0,"branchName":"test","updatedAt":updated,"team":{"id":"team"},"state":{"id":"s","name":"Todo","type":"unstarted"},"attachments":{"nodes":[]}})).unwrap()
}
fn page(nodes: Vec<Issue>, next: Option<&str>) -> Page {
    Page {
        nodes,
        page_info: PageInfo {
            has_next_page: next.is_some(),
            end_cursor: next.map(str::to_string),
        },
    }
}
#[test]
fn paginates_and_deduplicates_by_id_using_latest_updated_at() {
    let mut calls = 0;
    let issues = collect_pages(|cursor| {
        calls += 1;
        match calls {
            1 => {
                assert_eq!(cursor, None);
                Ok(page(vec![issue("a", "1")], Some("next")))
            }
            2 => {
                assert_eq!(cursor, Some("next"));
                Ok(page(vec![issue("a", "2"), issue("b", "1")], None))
            }
            _ => panic!("unexpected page"),
        }
    })
    .unwrap();
    assert_eq!(issues.len(), 2);
    assert_eq!(issues[0].updated_at, "2");
}
#[test]
fn handles_empty_page_and_errors_without_partial_snapshot() {
    assert!(collect_pages(|_| Ok(page(vec![], None)))
        .unwrap()
        .is_empty());
    let mut calls = 0;
    let result = collect_pages(|_| {
        calls += 1;
        if calls == 1 {
            Ok(page(vec![issue("a", "1")], Some("next")))
        } else {
            bail!("page failed")
        }
    });
    assert!(result.is_err());
}
#[test]
fn rejects_repeated_or_missing_cursors() {
    assert!(collect_pages(|_| Ok(page(vec![], Some("same")))).is_err());
    assert!(collect_pages(|_| Ok(Page {
        nodes: vec![],
        page_info: PageInfo {
            has_next_page: true,
            end_cursor: None
        }
    }))
    .is_err());
}
#[test]
fn mutation_requires_success_and_returned_state() {
    for value in [
        json!({"issueUpdate":{"success":false,"issue":null}}),
        json!({"issueUpdate":{"success":true,"issue":null}}),
    ] {
        assert!(updated_state(serde_json::from_value(value).unwrap()).is_err());
    }
    let state=updated_state(serde_json::from_value(json!({"issueUpdate":{"success":true,"issue":{"state":{"id":"done","name":"Done","type":"completed"}}}})).unwrap()).unwrap();
    assert_eq!(state.id, "done");
}
#[test]
fn http_error_bodies_are_surfaced_with_actionable_status_messages() {
    let body = r#"{"errors":[{"message":"Entity not found"}]}"#;
    let error = status_error(400, body).to_string();
    assert!(
        error.contains("HTTP 400") && error.contains("Entity not found"),
        "{error}"
    );
    let auth = status_error(401, r#"{"errors":[{"message":"bad key"}]}"#).to_string();
    assert!(
        auth.contains("Authentication failed") && auth.contains("bad key"),
        "{auth}"
    );
    let limited = status_error(429, "").to_string();
    assert!(limited.contains("Rate limited"), "{limited}");
    assert_ne!(auth, limited);
    assert!(auth.contains("check this Account's API key"));
    assert!(!auth.to_lowercase().contains("workspace"));
    for malformed in ["", "not json", "{}", r#"{"errors":"x"}"#] {
        assert_eq!(
            status_error(500, malformed).to_string(),
            "Linear HTTP 500: Linear request rejected."
        );
    }
}
#[test]
fn status_responses_are_read_through_http_response() {
    let response =
        ureq::Response::new(401, "Unauthorized", r#"{"errors":[{"message":"bad key"}]}"#).unwrap();
    let error = http_response(Err(ureq::Error::Status(401, response)))
        .unwrap_err()
        .to_string();
    assert!(
        error.contains("Authentication failed") && error.contains("bad key"),
        "{error}"
    );
}
#[test]
fn states_sort_by_workflow_type_then_position() {
    let state = |id: &str, kind: &str, position: f64| State {
        id: id.into(),
        name: id.into(),
        kind: kind.into(),
        position,
    };
    let mut states = vec![
        state("canceled", "canceled", 0.0),
        state("review", "started", 2.0),
        state("backlog", "backlog", 0.0),
        state("done", "completed", 0.0),
        state("progress", "started", 1.0),
        state("todo", "unstarted", 0.0),
    ];
    sort_states(&mut states);
    let ids: Vec<_> = states.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(
        ids,
        ["backlog", "todo", "progress", "review", "done", "canceled"]
    );
}
#[test]
fn in_review_requires_started_type() {
    let mut i = issue("a", "1");
    i.state.name = "In Review".into();
    i.state.kind = "started".into();
    assert!(i.is_in_review());
    i.state.kind = "completed".into();
    assert!(!i.is_in_review());
}
#[test]
fn graphql_errors_are_not_silently_ignored() {
    let response: Response<Value> =
        serde_json::from_value(json!({"data":{},"errors":[{"message":"denied"}]})).unwrap();
    assert!(decode(response).unwrap_err().to_string().contains("denied"));
}
#[test]
fn detects_prs_by_url_not_source_type_and_filters_terminal_states() {
    let mut i = issue("a", "1");
    i.attachments.nodes.push(Attachment {
        title: None,
        url: "https://github.com/o/r/pull/1".into(),
        source_type: Some("unknown".into()),
    });
    assert_eq!(i.pull_requests().count(), 1);
    i.state.kind = "completed".into();
    assert!(!i.is_open());
}
#[test]
fn assignee_decodes_including_unassigned() {
    let with = |assignee: Value| -> Issue {
        let mut value = json!({"id":"a","identifier":"HL-1","title":"test","url":"https://linear.app/test","priority":0,"branchName":"test","updatedAt":"1","team":{"id":"team"},"state":{"id":"s","name":"Todo","type":"unstarted"},"attachments":{"nodes":[]}});
        value["assignee"] = assignee;
        serde_json::from_value(value).unwrap()
    };
    let me = json!({"id":"u1","name":"Jinwoo Oh","displayName":"jinwoo","isMe":true});
    let issues = collect_pages(|_| Ok(page(vec![with(me.clone())], None))).unwrap();
    let assignee = issues[0].assignee.as_ref().unwrap();
    assert_eq!(assignee.id, "u1");
    assert_eq!(assignee.name, "Jinwoo Oh");
    assert_eq!(assignee.display_name, "jinwoo");
    assert!(assignee.is_me);

    let unassigned = collect_pages(|_| Ok(page(vec![with(Value::Null)], None))).unwrap();
    assert_eq!(unassigned.len(), 1);
    assert!(unassigned[0].assignee.is_none());
    // An issue decoded without the field at all is also Unassigned.
    assert!(issue("b", "1").assignee.is_none());
}
#[test]
fn assigned_issues_query_requests_the_assignee() {
    let assignee = QUERY
        .split("assignee {")
        .nth(1)
        .expect("query requests assignee");
    let fields = &assignee[..assignee.find('}').unwrap()];
    for field in ["id", "name", "displayName", "isMe"] {
        assert!(
            fields.split_whitespace().any(|f| f == field),
            "missing {field} in {fields}"
        );
    }
}
