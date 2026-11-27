use super::*;
use serde_json::json;

mod merge_tests;
mod refresh_tests;
mod scope_tests;

fn issue(id: &str, review: bool) -> linear::Issue {
    serde_json::from_value(json!({"id":id,"identifier":"HL-1","title":id,"url":"https://linear.app/test","priority":0,"branchName":"test","updatedAt":"2026-01-01T00:00:00Z","team":{"id":"team"},"state":{"id":"s","name":if review {"In Review"} else {"Todo"},"type":"started"},"attachments":{"nodes":[]}})).unwrap()
}
fn app() -> App {
    App::new(vec![
        AccountConfig {
            name: "work".into(),
            key: Err("work key unavailable".into()),
        },
        AccountConfig {
            name: "personal".into(),
            key: Err("personal key unavailable".into()),
        },
    ])
}
fn wait(app: &mut App, done: impl Fn(&App) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !done(app) {
        assert!(Instant::now() < deadline, "background result timed out");
        app.poll_background();
        std::thread::sleep(Duration::from_millis(1));
    }
}
#[test]
fn snapshot_preserves_selection_and_removes_absent_issues() {
    let mut app = app();
    let a = &mut app.accounts[0];
    a.list
        .apply_snapshot(vec![issue("a", false), issue("b", false)]);
    a.list.selection.select(Some(1));
    let mut b = issue("b", true);
    b.updated_at = "2026-01-02T00:00:00Z".into();
    a.list.apply_snapshot(vec![issue("a", false), b]);
    assert_eq!(a.list.selected().unwrap().id, "b");
    assert_eq!(a.list.selection.selected(), Some(0));
    a.list.apply_snapshot(vec![issue("a", false)]);
    assert_eq!(a.list.selected().unwrap().id, "a");
    a.list.apply_snapshot(vec![]);
    assert!(a.list.selected().is_none());
}
#[test]
fn all_accounts_queue_keeps_account_identity_and_review_order() {
    let mut app = app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("same", false)]);
    app.accounts[1]
        .list
        .apply_snapshot(vec![issue("same", true)]);
    app.poll_background();
    app.on_key(KeyCode::Char('a'));
    assert_eq!(app.queue.len(), 2);
    assert_eq!(app.selected().unwrap().0, 1);
    assert!(app.selected().unwrap().1.is_in_review());
    app.poll_background();
    assert_eq!(app.selected().unwrap().0, 1);
    app.on_key(KeyCode::Char('s'));
    assert_eq!(app.status.as_deref(), Some("personal key unavailable"));
}
#[test]
fn account_terms_render_and_existing_navigation_is_unchanged() {
    let mut app = app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("work-issue", false)]);
    app.accounts[1]
        .list
        .apply_snapshot(vec![issue("personal-issue", true)]);
    app.poll_background();
    assert!(!app.all_accounts_queue());
    app.on_key(KeyCode::Tab);
    assert_eq!(app.active, 1);
    app.on_key(KeyCode::Tab);
    assert!(app.all_accounts_queue());
    assert_eq!(app.selected().unwrap().0, 1);

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(70, 30)).unwrap();
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let text: String = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("All-accounts queue (2)"));
    assert!(text.contains("Account: personal"));
    assert!(text.contains("tab/1-9 Account"));
    assert!(!text.to_lowercase().contains("workspace"));

    app.on_key(KeyCode::Char('1'));
    assert_eq!(app.active, 0);
    assert!(!app.all_accounts_queue());
    app.on_key(KeyCode::BackTab);
    assert!(app.all_accounts_queue());
    app.on_key(KeyCode::Tab);
    assert_eq!(app.active, 0);
    app.on_key(KeyCode::Char('a'));
    assert!(app.all_accounts_queue());
}

#[test]
fn multiple_prs_show_picker_and_cancel_does_not_load() {
    let mut app = app();
    let mut i = issue("a", false);
    for n in 1..=2 {
        i.attachments.nodes.push(linear::Attachment {
            title: Some(format!("PR {n}")),
            url: format!("https://github.com/o/r/pull/{n}"),
            source_type: None,
        });
    }
    app.accounts[0].list.apply_snapshot(vec![i]);
    app.on_key(KeyCode::Enter);
    let View::PrPicker(p) = &app.view else {
        panic!("expected PR picker")
    };
    assert_eq!(p.options.len(), 2);
    app.on_key(KeyCode::Char('j'));
    let View::PrPicker(p) = &app.view else {
        panic!("expected PR picker")
    };
    assert_eq!(p.list.selected(), Some(1));
    app.on_key(KeyCode::Esc);
    assert!(matches!(app.view, View::Issues));
    assert!(app.loading_review().is_none());
}
#[test]
fn one_account_failure_does_not_erase_another_or_stale_data() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![issue("a", false)]);
    app.accounts[0].list.pending = Some(Job::spawn(|| anyhow::bail!("offline")).unwrap());
    wait(&mut app, |a| !a.accounts[0].list.loading());
    assert_eq!(app.accounts[0].list.issues.len(), 1);
    assert_eq!(app.accounts[0].list.error.as_deref(), Some("offline"));
    assert_eq!(app.queue.len(), 1);
}
#[test]
fn refresh_is_nonblocking_and_does_not_replace_an_active_request() {
    let mut app = app();
    let (release, waiter) = mpsc::channel();
    app.accounts[0].list.pending = Some(
        Job::spawn(move || {
            waiter.recv()?;
            Ok(vec![issue("a", false)])
        })
        .unwrap(),
    );
    app.on_key(KeyCode::Char('r'));
    app.on_key(KeyCode::Tab);
    assert_eq!(app.active, 1);
    app.poll_background();
    assert!(app.accounts[0].list.loading());
    release.send(()).unwrap();
    wait(&mut app, |a| !a.accounts[0].list.loading());
    assert_eq!(app.accounts[0].list.issues.len(), 1);
}
#[test]
fn mutation_result_routes_to_captured_account_even_after_switching() {
    let mut app = app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("same", false)]);
    app.accounts[1]
        .list
        .apply_snapshot(vec![issue("same", false)]);
    app.pending_mutation = Some(PendingMutation {
        account: 1,
        issue_id: "same".into(),
        title: "HL-1".into(),
        job: Job::spawn(|| {
            Ok(linear::State {
                id: "done".into(),
                name: "Done".into(),
                kind: "completed".into(),
                position: 0.0,
            })
        })
        .unwrap(),
    });
    wait(&mut app, |a| a.pending_mutation.is_none());
    assert_eq!(app.accounts[0].list.issues.len(), 1);
    assert!(app.accounts[1].list.issues.is_empty());
    assert!(app.status.as_ref().unwrap().contains("personal"));
}
#[test]
fn mutation_error_keeps_original_state() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![issue("a", false)]);
    app.pending_mutation = Some(PendingMutation {
        account: 0,
        issue_id: "a".into(),
        title: "HL-1".into(),
        job: Job::spawn(|| anyhow::bail!("denied")).unwrap(),
    });
    wait(&mut app, |a| a.pending_mutation.is_none());
    assert_eq!(app.accounts[0].list.issues[0].state.name, "Todo");
    assert!(app.status.as_ref().unwrap().contains("denied"));
}
#[test]
fn state_picker_cancel_never_submits_mutation() {
    let mut app = app();
    app.view = View::StatePicker(StatePicker {
        account: 1,
        issue_id: "a".into(),
        title: "HL-1".into(),
        options: vec![linear::State {
            id: "s".into(),
            name: "Done".into(),
            kind: "completed".into(),
            position: 0.0,
        }],
        started: Instant::now(),
        current_state: "s".into(),
        confirming: false,
        list: ListState::default(),
        pending: None,
        error: None,
    });
    app.on_key(KeyCode::Esc);
    assert!(app.pending_mutation.is_none());
    assert!(matches!(app.view, View::Issues));
}
#[test]
fn tiny_unicode_review_and_all_queue_render_without_panicking() {
    let mut app = app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("안녕", false)]);
    app.poll_background();
    for (width, height) in [(1, 1), (8, 3), (40, 10)] {
        let mut terminal =
            ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
        app.on_key(KeyCode::Char('a'));
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
        app.view = View::Review(Review::new(
            "한글".into(),
            review::parse("diff --git a/a b/a\n+한글\n"),
        ));
        app.on_key(KeyCode::Right);
        terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
        app.view = View::Issues;
    }
}
fn picker(app: &mut App) {
    let state = |id: &str, kind: &str| linear::State {
        id: id.into(),
        name: id.into(),
        kind: kind.into(),
        position: 0.0,
    };
    let mut list = ListState::default();
    list.select(Some(1));
    app.view = View::StatePicker(StatePicker {
        account: 0,
        issue_id: "a".into(),
        title: "HL-1".into(),
        options: vec![
            state("backlog", "backlog"),
            state("progress", "started"),
            state("done", "completed"),
        ],
        started: Instant::now(),
        current_state: "progress".into(),
        confirming: false,
        list,
        pending: None,
        error: None,
    });
}
#[test]
fn selecting_the_current_state_makes_no_mutation() {
    let mut app = app();
    picker(&mut app);
    app.on_key(KeyCode::Enter);
    assert!(app.pending_mutation.is_none());
    assert!(matches!(app.view, View::Issues));
}
#[test]
fn loaded_states_preselect_the_current_state() {
    let mut app = app();
    picker(&mut app);
    let View::StatePicker(p) = &mut app.view else {
        panic!()
    };
    let options = std::mem::take(&mut p.options);
    p.list.select(None);
    let (sender, job) = Job::channel();
    p.pending = Some(job);
    sender.send(Ok(options)).unwrap();
    app.poll_background();
    let View::StatePicker(p) = &app.view else {
        panic!()
    };
    assert_eq!(p.list.selected(), Some(1));
}
#[test]
fn terminal_states_require_confirmation_and_cancel_writes_nothing() {
    let mut app = app();
    picker(&mut app);
    app.on_key(KeyCode::Char('j'));
    app.on_key(KeyCode::Enter);
    let View::StatePicker(p) = &app.view else {
        panic!("expected confirmation")
    };
    assert!(p.confirming);
    assert!(app.pending_mutation.is_none());
    app.on_key(KeyCode::Char('n'));
    let View::StatePicker(p) = &app.view else {
        panic!()
    };
    assert!(!p.confirming);
    assert!(app.pending_mutation.is_none());
    app.on_key(KeyCode::Enter);
    app.on_key(KeyCode::Char('y'));
    // Account keys are unavailable in tests, so confirmation reaches the credential check.
    assert_eq!(app.status.as_deref(), Some("work key unavailable"));
}
#[test]
fn transient_status_clears_and_account_errors_stay_visible() {
    let mut app = app();
    app.accounts[0].list.error = Some("work: HTTP 401\nsecond line".into());
    app.status = Some("HL-1 has no linked pull request".into());
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(120, 20)).unwrap();
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(screen.contains("HTTP 401") && screen.contains("no linked pull request"));
    assert!(!screen.contains("second line"));
    app.on_key(KeyCode::Char('j'));
    assert!(app.status.is_none());
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(screen.contains("HTTP 401"));
}
#[test]
fn pr_view_footer_shows_status_and_bindings_at_sixty_columns() {
    let mut app = app();
    app.status = Some("PR changed while loading".into());
    app.view = View::Review(Review::new(
        "t".into(),
        review::parse("diff --git a/a b/a\n+x\n"),
    ));
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 20)).unwrap();
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(screen.contains("PR changed while loading"));
    assert!(screen.contains("f full/diff"));
    assert!(screen.contains("r status"));
    assert!(screen.contains("esc/q back") && screen.contains("Ctrl+C quit"));
    app.view = View::Issues;
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let screen = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    assert!(screen.contains("g/G ends") && screen.contains("tab/1-9 Account"));
}
#[test]
fn unknown_mutation_outcome_is_not_reported_as_failure() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![issue("a", false)]);
    let (sender, job) = Job::channel();
    app.pending_mutation = Some(PendingMutation {
        account: 0,
        issue_id: "a".into(),
        title: "HL-1".into(),
        job,
    });
    sender
        .send(Err(format!("{}: timed out", linear::UNKNOWN_OUTCOME)))
        .unwrap();
    app.poll_background();
    let status = app.status.clone().unwrap();
    assert!(
        status.contains("outcome unknown") && status.contains("refreshing"),
        "{status}"
    );
}
#[test]
fn worker_panics_become_errors() {
    let job: Job<()> = Job::spawn(|| panic!("boom")).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    let result = loop {
        if let Some(result) = job.poll() {
            break result;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(1));
    };
    assert_eq!(
        result.unwrap_err(),
        "background worker panicked; retry the request"
    );
}
#[test]
fn escape_quits_only_from_the_top_level() {
    let mut app = app();
    app.view = View::Review(Review::new(
        "t".into(),
        review::parse("diff --git a/a b/a\n+x\n"),
    ));
    app.on_key(KeyCode::Esc);
    assert!(matches!(app.view, View::Issues) && !app.quit);
    app.on_key(KeyCode::Esc);
    assert!(app.quit);
}
#[test]
fn ctrl_c_quits_from_any_view_but_plain_c_does_not() {
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
    let mut app = app();
    app.view = View::Review(Review::new(
        "t".into(),
        review::parse("diff --git a/a b/a\n+x\n"),
    ));
    app.on_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
    assert!(!app.quit);
    app.on_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
    assert!(app.quit);
}

fn open_loaded_pr(app: &mut App) {
    let (sender, result) = Job::channel();
    app.pending_review = Some(PendingReview {
        title: "HL-1 Test PR".into(),
        started: Instant::now(),
        result,
    });
    sender
        .send(Ok(super::tests::loaded(
            "diff --git a/a.rs b/a.rs\n+new\ndiff --git a/b.rs b/b.rs\n+other\n",
        )))
        .unwrap();
    app.poll_review();
}

fn status_job(app: &mut App) -> mpsc::Sender<Result<review::Readiness, String>> {
    assert!(app.pending_readiness.is_none());
    let (sender, result) = Job::channel();
    app.pending_readiness = Some(PendingReadiness {
        generation: app.review_generation,
        result,
    });
    let View::Review(review) = &mut app.view else {
        panic!("expected PR view")
    };
    review.readiness_started = Some(Instant::now());
    review.readiness_loading = true;
    sender
}

fn readiness(state: &str, draft: bool, mergeable: &str, decision: &str) -> review::Readiness {
    serde_json::from_value(json!({
        "headRefOid": "abcdef0123456789", "state": state, "isDraft": draft,
        "mergeable": mergeable, "reviewDecision": decision,
        "statusCheckRollup": [{"status":"COMPLETED", "conclusion":"SUCCESS"}]
    }))
    .unwrap()
}

#[test]
fn readiness_scheduling_is_nonblocking_and_refreshes_without_overlapping_jobs() {
    let mut app = app();
    open_loaded_pr(&mut app);
    let (release, blocked) = mpsc::channel();
    app.poll_readiness_with(|url| {
        assert_eq!(url, "https://github.com/o/r/pull/1");
        Job::spawn(move || {
            blocked.recv()?;
            Ok(readiness("OPEN", false, "MERGEABLE", "APPROVED"))
        })
    });
    app.on_key(KeyCode::Char('j'));
    app.on_key(KeyCode::Char('r'));
    assert!(render(&mut app).contains("+other"));
    app.poll_readiness_with(|_| panic!("must not start a second worker"));
    release.send(()).unwrap();
    wait(&mut app, |app| app.pending_readiness.is_none());
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(!review.readiness_loading);
    assert!(review.readiness.as_ref().unwrap().is_ok());
    app.poll_readiness_with(|_| panic!("status is still fresh"));

    // An explicit r refresh and the periodic refresh use the same bounded path.
    app.on_key(KeyCode::Char('r'));
    let (sender, job) = Job::channel();
    app.poll_readiness_with(|_| Ok(job));
    assert!(render(&mut app).contains("refreshing…"));
    sender.send(Err("refresh unavailable".into())).unwrap();
    app.poll_background();
    assert!(render(&mut app).contains("status failed: refresh unavailable"));
    let View::Review(review) = &mut app.view else {
        panic!()
    };
    review.readiness_started = Some(Instant::now() - REFRESH_INTERVAL);
    let (sender, job) = Job::channel();
    app.poll_readiness_with(|_| Ok(job));
    sender
        .send(Ok(readiness("CLOSED", false, "UNKNOWN", "")))
        .unwrap();
    app.poll_background();
    assert!(render(&mut app).contains("closed ·"));
}

#[test]
fn loaded_readiness_preserves_the_diff_snapshot_and_shows_pr_states() {
    for (state, draft, mergeable, decision, expected) in [
        (
            "OPEN",
            true,
            "CONFLICTING",
            "CHANGES_REQUESTED",
            "draft · conflicting · checks passed · review changes requested",
        ),
        (
            "MERGED",
            false,
            "UNKNOWN",
            "APPROVED",
            "merged · conflicts unknown · checks passed · review approved",
        ),
        (
            "CLOSED",
            false,
            "MERGEABLE",
            "",
            "closed · no conflicts · checks passed · review none",
        ),
        (
            "OPEN",
            false,
            "MERGEABLE",
            "REVIEW_REQUIRED",
            "open · no conflicts · checks passed · review required",
        ),
    ] {
        let mut app = app();
        open_loaded_pr(&mut app);
        let sender = status_job(&mut app);
        // A blocked status job neither masks the diff nor makes keys modal.
        app.poll_background();
        assert!(render(&mut app).contains("loading status"));
        app.on_key(KeyCode::Char('j'));
        let View::Review(review) = &app.view else {
            panic!()
        };
        assert_eq!(review.current().unwrap().path, "b.rs");
        sender
            .send(Ok(readiness(state, draft, mergeable, decision)))
            .unwrap();
        app.poll_background();
        let View::Review(review) = &app.view else {
            panic!()
        };
        assert!(!review.readiness_loading);
        assert!(
            review.readiness_line().contains(expected),
            "{}",
            review.readiness_line()
        );
        let snapshot = review.snapshot.as_deref().unwrap();
        assert_eq!(snapshot, &super::tests::loaded("").snapshot);
        assert_eq!(snapshot.head_sha, "abcdef0123456789");
        assert!(review.readiness_line().starts_with("head abcdef012345"));
        assert_eq!(review.files.len(), 2);
        assert!(render(&mut app).contains("+other"));
    }
}

#[test]
fn check_rollups_show_pending_failed_absent_and_unknown_without_guessing_success() {
    for (checks, expected) in [
        (
            json!([{"status":"COMPLETED","conclusion":"SUCCESS"}, {"state":"PENDING"}]),
            "pending",
        ),
        (
            json!([{"state":"SUCCESS"}, {"status":"COMPLETED","conclusion":"FAILURE"}]),
            "failed",
        ),
        (
            json!([{"status":"COMPLETED","conclusion":"TIMED_OUT"}]),
            "failed",
        ),
        (json!([{"status":"IN_PROGRESS"}]), "pending"),
        (json!([{"state":"ERROR"}]), "failed"),
        (
            json!([{"status":"COMPLETED","conclusion":"NEUTRAL"}, {"status":"COMPLETED","conclusion":"SKIPPED"}]),
            "passed",
        ),
        (json!([{"status":"COMPLETED","conclusion":null}]), "unknown"),
        (json!([{"state":"NEW_STATE"}]), "unknown"),
        (json!([]), "none"),
        (json!(null), "none"),
    ] {
        let mut app = app();
        open_loaded_pr(&mut app);
        let sender = status_job(&mut app);
        let status = serde_json::from_value(json!({
            "headRefOid":"abcdef0123456789", "state":"OPEN", "isDraft":false,
            "mergeable":"UNKNOWN", "reviewDecision":null, "statusCheckRollup":checks,
        }))
        .unwrap();
        sender.send(Ok(status)).unwrap();
        app.poll_background();
        assert!(render(&mut app).contains(&format!("checks {expected}")));
    }
}

#[test]
fn readiness_failure_shows_only_first_line_and_diff_remains_usable() {
    let mut app = app();
    open_loaded_pr(&mut app);
    let sender = status_job(&mut app);
    sender
        .send(Err(
            "gh failed: unavailable\nprivate diagnostic detail".into()
        ))
        .unwrap();
    app.poll_background();
    let text = render(&mut app);
    assert!(text.contains("status failed: gh failed: unavailable"));
    assert!(!text.contains("private diagnostic detail"));
    assert!(text.contains("+new"));
    app.on_key(KeyCode::Char('j'));
    app.on_key(KeyCode::Right);
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert_eq!(review.current().unwrap().path, "b.rs");
    assert!(review.horizontal > 0);
    // r requests another asynchronous status load, not a diff reload or write.
    app.on_key(KeyCode::Char('r'));
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(review.readiness_started.is_none());
    assert_eq!(review.files.len(), 2);
    assert!(app.pending_mutation.is_none());
}

#[test]
fn stale_readiness_is_discarded_even_when_reopening_the_same_head() {
    let mut app = app();
    open_loaded_pr(&mut app);
    let old = status_job(&mut app);
    let generation = app.review_generation;
    app.on_key(KeyCode::Esc);
    open_loaded_pr(&mut app);
    assert_ne!(app.review_generation, generation);
    // An old worker is retained until completion, bounding concurrency on reopen.
    app.poll_background();
    app.on_key(KeyCode::Char('r'));
    app.poll_background();
    assert_eq!(
        app.pending_readiness.as_ref().unwrap().generation,
        generation
    );
    let View::Review(review) = &mut app.view else {
        panic!()
    };
    // Suppress automatic network work in this fake-only test.
    review.readiness_started = Some(Instant::now());
    old.send(Ok(readiness("CLOSED", false, "UNKNOWN", "")))
        .unwrap();
    app.poll_background();
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(review.readiness.is_none());
    assert!(!render(&mut app).contains("closed ·"));
    let current = status_job(&mut app);
    current
        .send(Ok(readiness("OPEN", false, "MERGEABLE", "APPROVED")))
        .unwrap();
    app.poll_background();
    assert!(render(&mut app).contains("review approved"));
}

#[test]
fn changed_head_status_cannot_relabel_the_diff_snapshot() {
    let mut app = app();
    open_loaded_pr(&mut app);
    let sender = status_job(&mut app);
    let mut status = readiness("OPEN", false, "MERGEABLE", "APPROVED");
    status.head_ref_oid = "newer-head".into();
    sender.send(Ok(status)).unwrap();
    app.poll_background();
    let text = render(&mut app);
    assert!(text.contains("head abcdef012345"));
    assert!(text.contains("PR head changed; reopen PR to reload diff"));
    assert!(!text.contains("review approved") && !text.contains("newer-head"));
    assert!(text.contains("+new"));
}

#[test]
fn readiness_results_after_leaving_the_pr_view_are_ignored() {
    let mut app = app();
    open_loaded_pr(&mut app);
    let sender = status_job(&mut app);
    app.on_key(KeyCode::Esc);
    sender.send(Err("stale error".into())).unwrap();
    app.poll_background();
    assert!(matches!(app.view, View::Issues));
    assert!(app.status.is_none() && app.pending_readiness.is_none());
}

#[test]
fn readiness_and_all_pr_footer_keys_fit_at_sixty_columns() {
    let mut app = app();
    open_loaded_pr(&mut app);
    let sender = status_job(&mut app);
    sender
        .send(Ok(readiness(
            "OPEN",
            true,
            "CONFLICTING",
            "CHANGES_REQUESTED",
        )))
        .unwrap();
    app.poll_background();
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 24)).unwrap();
    terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
    let text = terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect::<String>();
    for label in [
        "head abcdef012345",
        "draft",
        "conflicting",
        "checks passed",
        "changes requested",
        "+new",
        "j/k file",
        "d/u scroll",
        "h/l horizontal",
        "f full/diff (toggle to retry)",
        "r status",
        "m merge",
        "esc/q back",
        "Ctrl+C quit",
    ] {
        assert!(text.contains(label), "missing {label}: {text}");
    }
}

fn render(app: &mut App) -> String {
    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(100, 30)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    terminal
        .backend()
        .buffer()
        .content
        .iter()
        .map(|c| c.symbol())
        .collect()
}
#[test]
fn full_file_and_state_loads_use_the_loading_box() {
    let mut app = app();
    let mut review = Review::new(
        "t".into(),
        review::parse("diff --git a/src/main.rs b/src/main.rs\n+x\n"),
    );
    let _sender = review.begin_test_file_load();
    app.view = View::Review(review);
    let text = render(&mut app);
    assert!(
        text.contains("Loading full file")
            && text.contains("src/main.rs")
            && text.contains('█')
            && text.contains("Full-file view")
    );

    picker(&mut app);
    let View::StatePicker(p) = &mut app.view else {
        panic!()
    };
    p.options.clear();
    let (_sender, job) = Job::channel();
    p.pending = Some(job);
    let text = render(&mut app);
    assert!(text.contains("Loading team states") && text.contains('█'));
    // A load error replaces the box with the message.
    let View::StatePicker(p) = &mut app.view else {
        panic!()
    };
    p.error = Some("Linear HTTP 401".into());
    let text = render(&mut app);
    assert!(text.contains("Linear HTTP 401") && !text.contains("Loading team states"));
}

fn assigned(id: &str, assignee: Option<(&str, &str, bool)>) -> linear::Issue {
    let mut issue = issue(id, false);
    issue.assignee = assignee.map(|(name, display_name, is_me)| linear::Assignee {
        id: format!("user-{display_name}"),
        name: name.into(),
        display_name: display_name.into(),
        is_me,
    });
    issue
}
fn buffer(app: &mut App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|f| ui::draw(f, app)).unwrap();
    terminal.backend().buffer().clone()
}
/// The row text on screen and the cell where `label` starts in it.
fn find(buffer: &ratatui::buffer::Buffer, row_text: &str, label: &str) -> (String, u16, u16) {
    let area = buffer.area;
    for y in 0..area.height {
        let line: String = (0..area.width).map(|x| buffer[(x, y)].symbol()).collect();
        if line.contains(row_text) {
            let start = line
                .find(label)
                .unwrap_or_else(|| panic!("{label} not in {line}"));
            let x = line[..start].chars().count() as u16;
            return (line, x, y);
        }
    }
    panic!("{row_text} not on screen")
}
#[test]
fn rows_show_you_display_name_or_unassigned() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![
        assigned("mine", Some(("Jinwoo Oh", "jinwoo", true))),
        assigned("theirs", Some(("Ada Lovelace", "ada", false))),
        assigned("nobody", None),
    ]);
    app.poll_background();
    let screen = buffer(&mut app, 160, 30);

    let (line, x, y) = find(&screen, "mine", "you ");
    assert!(!line.contains("jinwoo"), "{line}");
    assert_eq!(screen[(x, y)].fg, ratatui::style::Color::Green);
    assert!(screen[(x, y)]
        .modifier
        .contains(ratatui::style::Modifier::BOLD));

    let (_, x, y) = find(&screen, "theirs", "ada ");
    assert_eq!(screen[(x, y)].fg, ratatui::style::Color::Reset);

    let (_, x, y) = find(&screen, "nobody", "Unassigned");
    assert_eq!(screen[(x, y)].fg, ratatui::style::Color::DarkGray);
}
#[test]
fn assignee_column_is_fixed_width_and_keeps_titles_on_narrow_panes() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![
        assigned("title-one", Some(("Long", "김진우김진우김진우", false))),
        assigned("title-two", Some(("Short", "ada", false))),
        assigned("title-three", None),
    ]);
    app.poll_background();
    app.on_key(KeyCode::Char('a'));
    let screen = buffer(&mut app, 60, 30);
    let (one, x_one, _) = find(&screen, "title-one", "title-one");
    let (_, x_two, _) = find(&screen, "title-two", "title-two");
    let (_, x_three, _) = find(&screen, "title-three", "title-three");
    assert_eq!(x_one, x_two);
    assert_eq!(x_one, x_three);
    assert!(one.contains('…'), "long names are truncated: {one}");
}
/// Text of each row inside the list pane only, which spans `0..width` columns.
fn list_pane_rows(buffer: &ratatui::buffer::Buffer, width: u16) -> Vec<String> {
    (0..buffer.area.height)
        .map(|y| (0..width).map(|x| buffer[(x, y)].symbol()).collect())
        .collect()
}
#[test]
fn assignee_column_hides_when_it_would_crowd_out_titles() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![
        assigned("fix-login-redirect", Some(("Jinwoo Oh", "jinwoo", true))),
        assigned("add-billing-export", Some(("Ada Lovelace", "ada", false))),
        assigned("drop-legacy-flags", None),
    ]);
    app.poll_background();
    // 110 columns is the narrowest side-by-side layout (HL-34): the list gets 50 cells with
    // borders, 48 inside. An Account tab row fits identifier 9 + state 12 + assignee 11 and
    // a 16-cell title. The All-accounts queue's `[work] ` prefix would leave the title 9, so
    // the queue row drops the assignee.
    for (all, shown) in [(false, true), (true, false)] {
        if all {
            app.on_key(KeyCode::Char('a'));
        }
        let screen = buffer(&mut app, 110, 24);
        let rows = list_pane_rows(&screen, 50);
        for (title, label) in [
            ("fix-login-redire", "you"),
            ("add-billing-expo", "ada"),
            ("drop-legacy-flag", "Unassigned"),
        ] {
            let row = rows
                .iter()
                .find(|r| r.contains(title))
                .unwrap_or_else(|| panic!("{title} missing (queue: {all}): {rows:#?}"));
            assert_eq!(row.contains(label), shown, "{label} in {row} (queue: {all})");
        }
    }
    // The detail pane keeps the assignee.
    assert!(render(&mut app).contains("Assignee: "));
}
#[test]
fn assignee_column_shows_in_the_all_accounts_queue_on_wide_panes() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![
        assigned("mine", Some(("Jinwoo Oh", "jinwoo", true))),
        assigned("theirs", Some(("Ada Lovelace", "ada", false))),
        assigned("nobody", None),
    ]);
    app.poll_background();
    app.on_key(KeyCode::Char('a'));
    let screen = buffer(&mut app, 160, 30);
    let rows = list_pane_rows(&screen, 72);
    for (title, label) in [
        ("mine", "you "),
        ("theirs", "ada "),
        ("nobody", "Unassigned"),
    ] {
        let row = rows.iter().find(|r| r.contains(title)).unwrap();
        assert!(row.contains(label), "{label} not in {row}");
    }
}
#[test]
fn detail_pane_shows_assignee_full_name_or_unassigned() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![assigned(
        "theirs",
        Some(("Ada Lovelace", "ada", false)),
    )]);
    app.poll_background();
    assert!(render(&mut app).contains("Assignee: Ada Lovelace"));
    app.accounts[0]
        .list
        .apply_snapshot(vec![assigned("nobody", None)]);
    assert!(render(&mut app).contains("Assignee: Unassigned"));
}
#[test]
fn assignee_names_are_sanitized_like_titles() {
    let mut app = app();
    app.accounts[0].list.apply_snapshot(vec![assigned(
        "sneaky",
        Some(("Ada\u{202e}Lovelace", "\u{200b}x", false)),
    )]);
    app.poll_background();
    // Wide enough that the row keeps its assignee column.
    let screen = buffer(&mut app, 160, 30);
    let text: String = screen.content.iter().map(|c| c.symbol()).collect();
    assert!(text.contains("<U+200B>x"), "row name: {text}");
    assert!(
        text.contains("Assignee: Ada<U+202E>Lovelace"),
        "detail: {text}"
    );
}
