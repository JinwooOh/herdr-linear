use super::*;
use crate::merge::Submission;

mod failure_tests;

fn verify(app: &mut App, result: Result<review::Readiness, String>) {
    let (sender, job) = Job::channel();
    app.poll_merge_check_with(|url| {
        assert_eq!(url, "https://github.com/o/r/pull/1");
        Ok(job)
    });
    sender.send(result).unwrap();
    app.poll_merge_check_with(|_| panic!("must retain existing check"));
}

fn ready_app() -> App {
    let mut app = app();
    open_loaded_pr(&mut app);
    let sender = status_job(&mut app);
    sender
        .send(Ok(readiness("OPEN", false, "MERGEABLE", "APPROVED")))
        .unwrap();
    app.poll_readiness_with(|_| panic!("status is fresh"));
    app.merge_options.enable_merge = true;
    app.accounts[0].apply_snapshot(vec![issue("unchanged", true)]);
    app
}

fn picker(app: &App) -> &MergePicker {
    let View::Review(review) = &app.view else {
        panic!("expected PR view")
    };
    review.merge_picker.as_ref().expect("expected merge picker")
}

fn poll(app: &mut App) {
    app.poll_merge_with(
        |_| panic!("unexpected methods worker"),
        |_, _| panic!("unexpected merge submission"),
    );
}

fn load_methods(app: &mut App, options: Vec<MergeMethod>) {
    app.on_key(KeyCode::Char('m'));
    let (sender, job) = Job::channel();
    app.poll_merge_with(
        |snapshot| {
            assert_eq!(snapshot.url, "https://github.com/o/r/pull/1");
            Ok(job)
        },
        |_, _| panic!("loading methods must not merge"),
    );
    sender.send(Ok(options)).unwrap();
    poll(app);
}

#[test]
fn merge_is_off_by_default_and_only_available_in_pr_view() {
    let mut app = ready_app();
    app.merge_options = MergeOptions::default();
    app.on_key(KeyCode::Char('m'));
    poll(&mut app);
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(review.merge_picker.is_none());
    assert!(app.merge_request.is_none());
    assert!(app.pending_merge.is_none());
    app.on_key(KeyCode::Esc);
    app.merge_options.enable_merge = true;
    app.on_key(KeyCode::Char('m'));
    poll(&mut app);
    assert!(matches!(app.view, View::Issues));
}

#[test]
fn picker_only_lists_allowed_methods_and_preselects_configured_or_fallback() {
    use MergeMethod::*;
    for (preferred, allowed, selected) in [
        (Rebase, vec![Merge, Rebase], Rebase),
        (Merge, vec![Squash, Rebase], Squash),
        (Squash, vec![Merge, Rebase], Merge),
        (Rebase, vec![Squash], Squash),
    ] {
        let mut app = ready_app();
        app.merge_options.merge_method = preferred;
        load_methods(&mut app, allowed.clone());
        let p = picker(&app);
        assert_eq!(p.options, allowed);
        assert_eq!(p.options[p.list.selected().unwrap()], selected);
        let text = render(&mut app);
        for method in [Squash, Rebase] {
            // The word "merge" also appears in the heading/footer. The model
            // assertion above checks all methods, including merge commits.
            assert_eq!(text.contains(method.name()), allowed.contains(&method));
        }
        app.on_key(KeyCode::Char('j'));
        app.on_key(KeyCode::Char('k'));
        assert!(picker(&app).list.selected().unwrap() < allowed.len());
        assert!(app.pending_mutation.is_none());
    }
}

#[test]
fn loading_and_empty_or_failed_method_lists_cannot_submit() {
    for result in [
        Ok(vec![]),
        Err("private diagnostic must not be displayed".into()),
    ] {
        let mut app = ready_app();
        app.on_key(KeyCode::Char('m'));
        let (sender, job) = Job::channel();
        app.poll_merge_with(|_| Ok(job), |_, _| panic!("must not merge"));
        assert!(render(&mut app).contains("Loading allowed merge methods"));
        app.on_key(KeyCode::Enter);
        app.on_key(KeyCode::Char('y'));
        poll(&mut app);
        assert!(!picker(&app).confirming);
        sender.send(result).unwrap();
        poll(&mut app);
        assert!(picker(&app).options.is_empty());
        assert!(picker(&app).error.is_some());
        assert!(!render(&mut app).contains("private diagnostic"));
        app.on_key(KeyCode::Enter);
        app.on_key(KeyCode::Char('y'));
        poll(&mut app);
        assert!(app.merge_request.is_none());
        app.on_key(KeyCode::Esc);
        assert!(matches!(app.view, View::Review(_)));
    }
}

#[test]
fn confirmation_displays_identity_and_only_y_submits_cancel_preserves_diff() {
    for key in [
        KeyCode::Enter,
        KeyCode::Char('n'),
        KeyCode::Char('Y'),
        KeyCode::Esc,
        KeyCode::Char('q'),
        KeyCode::Down,
        KeyCode::Backspace,
    ] {
        let mut app = ready_app();
        load_methods(&mut app, vec![MergeMethod::Merge, MergeMethod::Squash]);
        app.on_key(KeyCode::Char('y')); // y in the picker does nothing.
        poll(&mut app);
        assert!(!picker(&app).confirming);
        app.on_key(KeyCode::Enter);
        assert!(picker(&app).confirming);
        let title = super::super::tests::loaded("").snapshot.title;
        let text = render(&mut app);
        for label in [
            "Confirm merge",
            &title,
            "PR #1",
            "o/r",
            "Method: squash",
            "y merge",
        ] {
            assert!(text.contains(label), "missing {label}: {text}");
        }
        app.on_key(key);
        assert!(!picker(&app).confirming);
        poll(&mut app);
        assert!(app.merge_request.is_none());
        app.on_key(KeyCode::Esc);
        poll(&mut app);
        let View::Review(review) = &app.view else {
            panic!()
        };
        assert!(review.merge_picker.is_none());
        assert_eq!(review.files.len(), 2);
        assert!(render(&mut app).contains("+new"));
        assert!(app.pending_mutation.is_none());
        assert_eq!(app.accounts[0].issues[0].state.name, "In Review");
    }
}

#[test]
fn explicit_confirmation_queues_pinned_background_merge_and_success_stays_in_pr_view() {
    let mut app = ready_app();
    app.merge_options.merge_method = MergeMethod::Rebase;
    load_methods(&mut app, vec![MergeMethod::Squash, MergeMethod::Rebase]);
    let expected = match &app.view {
        View::Review(r) => r.snapshot.as_deref().unwrap().clone(),
        _ => panic!(),
    };
    app.on_key(KeyCode::Enter);
    app.on_key(KeyCode::Char('y'));
    // Keys only queue work; the poll path starts the injected background job.
    assert!(app.pending_merge.is_none());
    assert!(app.merge_request.is_some());
    let (sender, job) = Job::channel();
    app.poll_merge_with(
        |_| panic!("methods already loaded"),
        |snapshot, method| {
            assert_eq!(snapshot, expected);
            assert_eq!(snapshot.head_sha, "abcdef0123456789");
            assert_eq!(method, MergeMethod::Rebase);
            Ok(job)
        },
    );
    app.on_key(KeyCode::Char('m'));
    poll(&mut app);
    assert!(app.pending_merge.is_some());
    assert!(app.merge_request.is_none());
    app.on_key(KeyCode::Char('j'));
    let text = render(&mut app);
    assert!(text.contains("Merging…") && text.contains("+other"));
    sender.send(Ok(Submission::Accepted)).unwrap();
    poll(&mut app);
    assert!(!render(&mut app).contains("Merged · merging disabled"));
    verify(&mut app, Ok(readiness("MERGED", false, "UNKNOWN", "")));
    assert!(matches!(app.view, View::Review(_)));
    assert!(render(&mut app).contains("Merged · merging disabled"));
    app.on_key(KeyCode::Char('m'));
    poll(&mut app);
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(review.merged && review.merge_picker.is_none());
    assert!(!review.can_merge());
    app.poll_readiness_with(|_| panic!("must not refresh a known merged PR"));
    assert!(app.pending_mutation.is_none());
    assert_eq!(app.accounts[0].issues[0].state.name, "In Review");
    app.on_key(KeyCode::Esc);
    assert!(matches!(app.view, View::Issues));
    assert!(!app.quit);
}

#[test]
fn known_unsafe_or_unreadable_prs_cannot_open_merge_picker() {
    for (state, draft, mergeable) in [
        ("CLOSED", false, "MERGEABLE"),
        ("MERGED", false, "MERGEABLE"),
        ("OPEN", true, "MERGEABLE"),
        ("OPEN", false, "CONFLICTING"),
    ] {
        let mut app = ready_app();
        let View::Review(review) = &mut app.view else {
            panic!()
        };
        review.readiness = Some(Ok(readiness(state, draft, mergeable, "")));
        app.on_key(KeyCode::Char('m'));
        poll(&mut app);
        let View::Review(review) = &app.view else {
            panic!()
        };
        assert!(review.merge_picker.is_none());
    }
    let mut app = ready_app();
    let View::Review(review) = &mut app.view else {
        panic!()
    };
    review.readiness = Some(Err("unavailable".into()));
    app.on_key(KeyCode::Char('m'));
    poll(&mut app);
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(review.merge_picker.is_none());
}

#[test]
fn cancelled_method_load_is_bounded_and_late_results_do_not_change_another_pr() {
    let mut app = ready_app();
    app.on_key(KeyCode::Char('m'));
    let (sender, job) = Job::channel();
    app.poll_merge_with(|_| Ok(job), |_, _| panic!("must not merge"));
    app.on_key(KeyCode::Esc); // picker -> PR view
    app.on_key(KeyCode::Esc); // PR view -> issues
    open_loaded_pr(&mut app);
    let View::Review(review) = &mut app.view else {
        panic!()
    };
    review.snapshot.as_mut().unwrap().url = "https://github.com/o/other/pull/2".into();
    poll(&mut app); // Retains the old job, never starts another concurrently.
    sender.send(Ok(vec![MergeMethod::Squash])).unwrap();
    poll(&mut app);
    assert!(app.pending_methods.is_none());
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(review.merge_picker.is_none());
    assert!(!review.merged);
}

#[test]
fn merge_result_follows_confirmed_identity_not_current_pr() {
    for same_snapshot in [false, true] {
        let mut app = ready_app();
        load_methods(&mut app, vec![MergeMethod::Squash]);
        app.on_key(KeyCode::Enter);
        app.on_key(KeyCode::Char('y'));
        let (sender, job) = Job::channel();
        app.poll_merge_with(|_| panic!(), |_, _| Ok(job));
        app.on_key(KeyCode::Esc);
        open_loaded_pr(&mut app);
        if !same_snapshot {
            let View::Review(review) = &mut app.view else {
                panic!()
            };
            let snapshot = review.snapshot.as_mut().unwrap();
            snapshot.url = "https://github.com/o/r/pull/2".into();
            snapshot.number = 2;
        }
        app.on_key(KeyCode::Char('m'));
        poll(&mut app); // Cannot submit a second merge while one is outstanding.
        sender.send(Ok(Submission::Accepted)).unwrap();
        poll(&mut app);
        verify(&mut app, Ok(readiness("MERGED", false, "UNKNOWN", "")));
        let View::Review(review) = &app.view else {
            panic!()
        };
        assert_eq!(review.merged, same_snapshot);
        assert!(review.merge_picker.is_none());
        assert!(app.pending_mutation.is_none());
    }
}

#[test]
fn uncertain_merge_outcome_never_claims_success_and_requires_new_status() {
    let mut app = ready_app();
    load_methods(&mut app, vec![MergeMethod::Squash]);
    app.on_key(KeyCode::Enter);
    app.on_key(KeyCode::Char('y'));
    let (sender, job) = Job::channel();
    app.poll_merge_with(|_| panic!(), |_, _| Ok(job));
    sender
        .send(Err("timeout with private subprocess details".into()))
        .unwrap();
    poll(&mut app);
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(!review.merged);
    assert!(review.readiness.is_none());
    assert!(!review.can_merge());
    let text = render(&mut app);
    assert!(text.contains("Merge outcome unknown"));
    assert!(!text.contains("private subprocess details"));
    app.on_key(KeyCode::Char('m'));
    poll(&mut app);
    let View::Review(review) = &app.view else {
        panic!()
    };
    assert!(review.merge_picker.is_none());
    assert!(app.pending_mutation.is_none());
}

#[test]
fn merge_picker_and_confirmation_render_in_tiny_unicode_panes() {
    let mut app = ready_app();
    let View::Review(review) = &mut app.view else {
        panic!()
    };
    review.snapshot.as_mut().unwrap().title = "한글 PR \u{202e} control".into();
    load_methods(&mut app, vec![MergeMethod::Squash]);
    for confirming in [false, true] {
        if confirming {
            app.on_key(KeyCode::Enter);
        }
        for (width, height) in [(1, 1), (20, 5), (60, 20)] {
            let mut terminal =
                ratatui::Terminal::new(ratatui::backend::TestBackend::new(width, height)).unwrap();
            terminal.draw(|f| ui::draw(f, &mut app)).unwrap();
        }
    }
    let text = render(&mut app);
    assert!(text.contains("<U+202E> control"));
    assert!(!text.contains('\u{202e}'));
}
