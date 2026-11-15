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

fn pending(app: &mut App) -> mpsc::Sender<Result<Vec<review::FileDiff>, String>> {
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
        .send(Ok(review::parse("diff --git a/a.rs b/a.rs\n+new\n")))
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
    assert!(old.send(Ok(Vec::new())).is_err());
    app.poll_review();
    assert!(matches!(app.view, View::Issues));
}

#[test]
fn slow_loader_runs_off_the_ui_thread_and_repeated_enter_is_ignored() {
    let mut app = app();
    let (release, wait) = mpsc::channel();
    app.start_review("Slow PR".into(), move || {
        wait.recv().unwrap();
        Ok(Vec::new())
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
    assert!(old.send(Ok(Vec::new())).is_err());
    let new = pending(&mut app);
    new.send(Err("new request failed".into())).unwrap();
    app.poll_review();
    assert!(matches!(app.view, View::Issues));
    assert_eq!(app.status.as_deref(), Some("new request failed"));
}
