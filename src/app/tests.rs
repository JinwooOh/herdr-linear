use super::*;

fn app() -> App {
    // Missing credentials deliberately avoid network requests in tests.
    App::new(vec![
        AccountConfig {
            name: "work".into(),
            key: Err("test account".into()),
        },
        AccountConfig {
            name: "personal".into(),
            key: Err("test account".into()),
        },
    ])
}

pub(super) fn loaded(diff: &str) -> review::LoadedReview {
    review::LoadedReview {
        snapshot: review::PrSnapshot {
            original_url: "https://github.com/o/r/pull/1".into(),
            url: "https://github.com/o/r/pull/1".into(),
            repository: "o/r".into(),
            number: 1,
            title: "Test PR".into(),
            head_sha: "abcdef0123456789".into(),
            base_sha: "base".into(),
        },
        files: review::parse(diff),
    }
}

fn pending(app: &mut App) -> mpsc::Sender<Result<review::LoadedReview, String>> {
    let (sender, result) = Job::channel();
    app.pending_review = Some(PendingReview {
        title: "Test PR".into(),
        started: Instant::now(),
        result,
    });
    sender
}

#[test]
fn completed_load_opens_review() {
    let mut app = app();
    let sender = pending(&mut app);
    sender
        .send(Ok(loaded("diff --git a/a.rs b/a.rs\n+new\n")))
        .unwrap();
    app.poll_review();
    let View::Review(review) = &app.view else {
        panic!("expected review")
    };
    assert_eq!(review.title, "Test PR");
    assert_eq!(review.files[0].path, "a.rs");
    assert!(app.loading_review().is_none());
}

#[test]
fn failure_preserves_issue_list_and_reports_error() {
    let mut app = app();
    let sender = pending(&mut app);
    sender.send(Err("gh failed".into())).unwrap();
    app.poll_review();
    assert!(matches!(app.view, View::Issues));
    assert_eq!(app.accounts.len(), 2);
    assert_eq!(app.status.as_deref(), Some("gh failed"));
    assert!(app.loading_review().is_none());
}

#[test]
fn disconnected_worker_reports_error() {
    let mut app = app();
    drop(pending(&mut app));
    app.poll_review();
    assert_eq!(
        app.status.as_deref(),
        Some("background worker stopped unexpectedly")
    );
    assert!(app.loading_review().is_none());
}

#[test]
fn escape_cancels_without_quitting_and_discards_late_result() {
    let mut app = app();
    let old = pending(&mut app);
    app.on_key(KeyCode::Esc);
    assert!(!app.quit);
    assert!(app.loading_review().is_none());
    assert!(old.send(Ok(loaded(""))).is_err());
    app.poll_review();
    assert!(matches!(app.view, View::Issues));
}

#[test]
fn slow_loader_runs_off_the_ui_thread_and_repeated_enter_is_ignored() {
    let mut app = app();
    let (release, wait) = mpsc::channel();
    app.start_review("Slow PR".into(), move || {
        wait.recv().unwrap();
        Ok(loaded(""))
    });
    app.poll_review();
    app.on_key(KeyCode::Enter);
    assert_eq!(app.loading_review(), Some("Slow PR"));
    assert!(matches!(app.view, View::Issues));
    release.send(()).unwrap();
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while app.loading_review().is_some() {
        assert!(
            std::time::Instant::now() < deadline,
            "loader did not finish"
        );
        app.poll_review();
        std::thread::sleep(Duration::from_millis(1));
    }
    assert!(matches!(app.view, View::Review(_)));
}

fn screen(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal =
        ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
    terminal.draw(|frame| ui::draw(frame, app)).unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn loading_box_is_centered_over_the_issue_list() {
    let mut app = app();
    let _sender = pending(&mut app);
    let text = screen(&mut app, 100, 30);
    assert!(text.contains("Loading pull request"));
    assert!(text.contains("Test PR"));
    assert!(text.contains('█') && text.contains('░'));
    assert!(text.contains("esc cancel"));
    // Roughly in the middle of the pane, not tucked into the footer.
    let rows: Vec<_> = text.lines().collect();
    let row = rows
        .iter()
        .position(|l| l.contains("Loading pull request"))
        .unwrap();
    assert!((5..20).contains(&row), "title row {row}");
    // The normal footer still lists the keys.
    assert!(text.contains("g/G") && text.contains("1-9"));
}

#[test]
fn slow_loads_add_reassurance() {
    let mut app = app();
    let _sender = pending(&mut app);
    assert!(!screen(&mut app, 100, 30).contains("Still working"));
    app.pending_review.as_mut().unwrap().started -= Duration::from_secs(5);
    let text = screen(&mut app, 100, 30);
    assert!(text.contains("Still working") && text.contains("5s"));
}

#[test]
fn loading_box_survives_tiny_panes() {
    for (w, h) in [(1, 1), (5, 3), (20, 6), (60, 12)] {
        let mut app = app();
        let _sender = pending(&mut app);
        screen(&mut app, w, h);
    }
}

#[test]
fn keys_other_than_cancel_and_quit_are_ignored_while_loading() {
    for code in [
        KeyCode::Char('j'),
        KeyCode::Down,
        KeyCode::Char('G'),
        KeyCode::Tab,
        KeyCode::Char('r'),
        KeyCode::Enter,
    ] {
        let mut app = app();
        let _sender = pending(&mut app);
        app.on_key(code);
        assert!(
            app.loading_review().is_some(),
            "{code:?} cancelled the load"
        );
        assert_eq!(app.active, 0, "{code:?} moved behind the loading box");
        assert!(!app.quit);
    }
    let mut app = app();
    let _sender = pending(&mut app);
    app.on_key(KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn cancelled_load_cannot_open_later_and_a_new_load_works() {
    let mut app = app();
    let old = pending(&mut app);
    app.on_key(KeyCode::Esc);
    app.on_key(KeyCode::Tab);
    assert_eq!(app.active, 1);
    assert!(old.send(Ok(loaded(""))).is_err());
    let new = pending(&mut app);
    new.send(Err("new request failed".into())).unwrap();
    app.poll_review();
    assert!(matches!(app.view, View::Issues));
    assert_eq!(app.status.as_deref(), Some("new request failed"));
}

/// The All-accounts queue with one `personal` issue whose title is longer than the row.
fn queue_with_long_title() -> App {
    let mut app = app();
    app.accounts[1].list.apply_snapshot(vec![serde_json::from_value(serde_json::json!({"id":"i","identifier":"HL-1","title":"abcdefghijklmnopqrstuvwxyz","url":"https://linear.app/test","priority":0,"branchName":"b","updatedAt":"2026-01-01T00:00:00Z","team":{"id":"team"},"state":{"id":"s","name":"Todo","type":"started"},"attachments":{"nodes":[]}})).unwrap()]);
    app.poll_background();
    app.on_key(KeyCode::Char('a'));
    app
}

/// Side by side puts the Detail pane's title on the top row, beside the tab bar.
fn side_by_side(text: &str) -> bool {
    text.lines().next().unwrap().contains(" Detail ")
}

#[test]
fn panes_stack_below_the_cutoff_that_keeps_queue_titles_16_cells() {
    // "[personal] " 11 + identifier 9 + state 12 + title 16 = 48 cells inside the borders,
    // so the list needs 50 columns. 45% of 110 is 49.5, which the layout rounds to 50;
    // 45% of 109 is 49.05, which gives 49.
    let mut app = queue_with_long_title();
    let text = screen(&mut app, 109, 30);
    assert!(!side_by_side(&text), "stacked at 109:\n{text}");
    // Stacked rows use the full width, so the assignee and the whole title show.
    assert!(
        text.contains("[personal] HL-1     Todo        Unassigned abcdefghijklmnopqrstuvwxyz"),
        "full-width row at 109:\n{text}"
    );

    let text = screen(&mut app, 110, 30);
    assert!(side_by_side(&text), "side by side at 110:\n{text}");
    // The list keeps its 45% share: the row ends after exactly 16 title cells.
    assert!(
        text.contains("│[personal] HL-1     Todo        abcdefghijklmnop││"),
        "16 title cells at 110:\n{text}"
    );
}

#[test]
fn the_cutoff_follows_the_longest_account_name_on_every_account_tab() {
    let mut app = App::new(
        ["a", "longername"]
            .map(|name| AccountConfig {
                name: name.into(),
                key: Err("test account".into()),
            })
            .into(),
    );
    // "[longername] " 13 + 9 + 12 + 16 = 50 inside the borders, so the list needs 52.
    // 45% of 115 is 51.75 (52); 45% of 114 is 51.3 (51).
    for active in [0, 1, 2] {
        app.active = active;
        assert!(!side_by_side(&screen(&mut app, 114, 30)), "tab {active} at 114");
        assert!(side_by_side(&screen(&mut app, 115, 30)), "tab {active} at 115");
    }
}

/// One Account whose configured name hides a right-to-left override, with one long-title issue.
fn account_name_with_control_character() -> App {
    let mut app = App::new(
        [AccountConfig {
            name: "a\u{202e}".into(),
            key: Err("test account".into()),
        }]
        .into(),
    );
    app.accounts[0].list.apply_snapshot(vec![serde_json::from_value(serde_json::json!({"id":"i","identifier":"HL-1","title":"abcdefghijklmnopqrstuvwxyz","url":"https://linear.app/test","priority":0,"branchName":"b","updatedAt":"2026-01-01T00:00:00Z","team":{"id":"team"},"state":{"id":"s","name":"Todo","type":"started"},"attachments":{"nodes":[]}})).unwrap()]);
    app.poll_background();
    app
}

#[test]
fn the_cutoff_measures_the_sanitized_account_prefix_that_queue_rows_draw() {
    let mut app = account_name_with_control_character();
    app.on_key(KeyCode::Char('a'));
    // "[a<U+202E>] " 12 + 9 + 12 + 16 = 49 inside the borders, so the list needs 51.
    // 45% of 113 is 50.85 (51); 45% of 112 is 50.4 (50).
    assert!(!side_by_side(&screen(&mut app, 112, 30)), "stacked at 112");
    let text = screen(&mut app, 113, 30);
    assert!(side_by_side(&text), "side by side at 113:\n{text}");
    assert!(
        text.contains("│[a<U+202E>] HL-1     Todo        abcdefghijklmnop││"),
        "sanitized prefix and 16 title cells at 113:\n{text}"
    );
}

#[test]
fn the_account_tab_bar_shows_sanitized_account_names() {
    let mut app = account_name_with_control_character();
    let text = screen(&mut app, 120, 30);
    let tabs = text.lines().next().unwrap();
    assert!(tabs.contains(" a<U+202E> (1) "), "tab bar: {tabs}");
}

#[test]
fn the_detail_pane_shows_the_sanitized_account_name() {
    let mut app = account_name_with_control_character();
    let text = screen(&mut app, 120, 30);
    assert!(text.contains("│Account: a<U+202E>  "), "detail pane:\n{text}");
}

#[test]
fn the_state_picker_heading_shows_the_sanitized_account_name_and_issue() {
    let mut app = account_name_with_control_character();
    app.view = View::StatePicker(StatePicker {
        account: 0,
        issue_id: "i".into(),
        title: "HL\u{200b}1".into(),
        options: Vec::new(),
        current_state: "s".into(),
        started: Instant::now(),
        confirming: false,
        list: ListState::default(),
        pending: None,
        error: None,
    });
    let text = screen(&mut app, 120, 30);
    let heading = text.lines().next().unwrap();
    assert!(
        heading.contains(" a<U+202E> / HL<U+200B>1 — change state "),
        "heading: {heading}"
    );
}

#[test]
fn the_pr_picker_heading_shows_the_sanitized_issue() {
    let mut app = account_name_with_control_character();
    app.view = View::PrPicker(PrPicker {
        title: "HL\u{200b}1".into(),
        options: (1..=2)
            .map(|n| linear::Attachment {
                title: Some(format!("PR {n}")),
                url: format!("https://github.com/o/r/pull/{n}"),
                source_type: None,
            })
            .collect(),
        list: ListState::default(),
    });
    let text = screen(&mut app, 120, 30);
    let heading = text.lines().next().unwrap();
    assert!(
        heading.contains(" HL<U+200B>1 — choose PR "),
        "heading: {heading}"
    );
}

#[test]
fn panes_stack_under_80_columns_even_with_a_one_letter_account() {
    let mut app = App::new(
        [AccountConfig {
            name: "x".into(),
            key: Err("test account".into()),
        }]
        .into(),
    );
    for width in [60, 79] {
        assert!(!side_by_side(&screen(&mut app, width, 30)), "at {width}");
    }
}
