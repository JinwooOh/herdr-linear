use super::*;
use serde_json::json;

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
    a.apply_snapshot(vec![issue("a", false), issue("b", false)]);
    a.list.select(Some(1));
    let mut b = issue("b", true);
    b.updated_at = "2026-01-02T00:00:00Z".into();
    a.apply_snapshot(vec![issue("a", false), b]);
    assert_eq!(a.selected().unwrap().id, "b");
    assert_eq!(a.list.selected(), Some(0));
    a.apply_snapshot(vec![issue("a", false)]);
    assert_eq!(a.selected().unwrap().id, "a");
    a.apply_snapshot(vec![]);
    assert!(a.selected().is_none());
}
#[test]
fn all_accounts_queue_keeps_account_identity_and_review_order() {
    let mut app = app();
    app.accounts[0].apply_snapshot(vec![issue("same", false)]);
    app.accounts[1].apply_snapshot(vec![issue("same", true)]);
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
    app.accounts[0].apply_snapshot(vec![issue("work-issue", false)]);
    app.accounts[1].apply_snapshot(vec![issue("personal-issue", true)]);
    app.poll_background();
    assert!(!app.all_accounts_queue());
    app.on_key(KeyCode::Tab);
    assert_eq!(app.active, 1);
    app.on_key(KeyCode::Tab);
    assert!(app.all_accounts_queue());
    assert_eq!(app.selected().unwrap().0, 1);

    let mut terminal = ratatui::Terminal::new(ratatui::backend::TestBackend::new(60, 30)).unwrap();
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
    app.accounts[0].apply_snapshot(vec![i]);
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
    app.accounts[0].apply_snapshot(vec![issue("a", false)]);
    app.accounts[0].pending = Some(Job::spawn(|| anyhow::bail!("offline")).unwrap());
    wait(&mut app, |a| !a.accounts[0].loading());
    assert_eq!(app.accounts[0].issues.len(), 1);
    assert_eq!(app.accounts[0].error.as_deref(), Some("offline"));
    assert_eq!(app.queue.len(), 1);
}
#[test]
fn refresh_is_nonblocking_and_does_not_replace_an_active_request() {
    let mut app = app();
    let (release, waiter) = mpsc::channel();
    app.accounts[0].pending = Some(
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
    assert!(app.accounts[0].loading());
    release.send(()).unwrap();
    wait(&mut app, |a| !a.accounts[0].loading());
    assert_eq!(app.accounts[0].issues.len(), 1);
}
#[test]
fn mutation_result_routes_to_captured_account_even_after_switching() {
    let mut app = app();
    app.accounts[0].apply_snapshot(vec![issue("same", false)]);
    app.accounts[1].apply_snapshot(vec![issue("same", false)]);
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
    assert_eq!(app.accounts[0].issues.len(), 1);
    assert!(app.accounts[1].issues.is_empty());
    assert!(app.status.as_ref().unwrap().contains("personal"));
}
#[test]
fn mutation_error_keeps_original_state() {
    let mut app = app();
    app.accounts[0].apply_snapshot(vec![issue("a", false)]);
    app.pending_mutation = Some(PendingMutation {
        account: 0,
        issue_id: "a".into(),
        title: "HL-1".into(),
        job: Job::spawn(|| anyhow::bail!("denied")).unwrap(),
    });
    wait(&mut app, |a| a.pending_mutation.is_none());
    assert_eq!(app.accounts[0].issues[0].state.name, "Todo");
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
    app.accounts[0].apply_snapshot(vec![issue("안녕", false)]);
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
    app.accounts[0].error = Some("work: HTTP 401\nsecond line".into());
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
    app.accounts[0].apply_snapshot(vec![issue("a", false)]);
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
