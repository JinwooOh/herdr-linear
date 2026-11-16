use super::*;

fn submit(app: &mut App) -> mpsc::Sender<Result<Submission, String>> {
    load_methods(app, vec![MergeMethod::Squash]);
    app.on_key(KeyCode::Enter);
    app.on_key(KeyCode::Char('y'));
    let (sender, job) = Job::channel();
    app.poll_merge_with(
        |_| panic!("methods loaded"),
        |snapshot, _| {
            assert_eq!(snapshot.head_sha, "abcdef0123456789");
            Ok(job)
        },
    );
    sender
}

fn review(app: &App) -> &Review {
    let View::Review(review) = &app.view else {
        panic!("expected PR view")
    };
    review
}

#[test]
fn normal_quit_keys_keep_submitted_merges_and_verification_supervised() {
    use ratatui::crossterm::event::{KeyEvent, KeyModifiers};
    fn assert_cannot_quit(app: &mut App) {
        app.view = View::Issues;
        for code in [KeyCode::Char('q'), KeyCode::Esc] {
            app.on_key(code);
            assert!(!app.quit);
            assert!(app
                .status
                .as_deref()
                .unwrap()
                .contains("wait for outcome verification"));
        }
        app.on_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL));
        assert!(!app.quit);
        assert!(app
            .status
            .as_deref()
            .unwrap()
            .contains("wait for outcome verification"));
    }

    let mut app = ready_app();
    load_methods(&mut app, vec![MergeMethod::Squash]);
    app.on_key(KeyCode::Enter);
    app.on_key(KeyCode::Char('y'));
    assert!(app.merge_request.is_some());
    assert_cannot_quit(&mut app);
    let (sender, job) = Job::channel();
    app.poll_merge_with(|_| panic!("methods already loaded"), |_, _| Ok(job));
    assert!(app.pending_merge.is_some());
    assert_cannot_quit(&mut app);
    sender.send(Ok(Submission::Accepted)).unwrap();
    poll(&mut app);
    assert!(app.pending_merge_check.is_some());
    assert_cannot_quit(&mut app);
    verify(
        &mut app,
        Ok(readiness("MERGED", false, "MERGEABLE", "APPROVED")),
    );
    assert!(app.pending_merge_check.is_none());
    app.on_key(KeyCode::Char('q'));
    assert!(app.quit);
}

#[test]
fn head_mismatch_never_verifies_or_retries_a_write_or_replaces_the_diff() {
    let mut app = ready_app();
    let sender = submit(&mut app);
    let snapshot = review(&app).snapshot.clone();
    sender.send(Ok(Submission::Changed)).unwrap();
    poll(&mut app);
    assert!(render(&mut app).contains("PR changed, reload"));
    assert!(!review(&app).merged && !review(&app).can_merge());
    assert!(app.pending_merge_check.is_none());
    app.on_key(KeyCode::Char('m'));
    assert!(app
        .status
        .as_deref()
        .unwrap()
        .contains("PR changed, reload"));
    assert!(review(&app).merge_picker.is_none());
    let sender = status_job(&mut app);
    let mut changed = readiness("OPEN", false, "MERGEABLE", "");
    changed.head_ref_oid = "fedcba9876543210".into();
    sender.send(Ok(changed)).unwrap();
    app.poll_readiness_with(|_| panic!("status is fresh"));
    poll(&mut app);
    assert_eq!(review(&app).snapshot, snapshot);
    assert_eq!(review(&app).files.len(), 2);
    assert!(!review(&app).can_merge());
}

#[test]
fn unavailable_prs_explain_the_reason_at_the_app_key_seam() {
    for (state, draft, mergeable, reason) in [
        ("OPEN", true, "MERGEABLE", "draft"),
        ("MERGED", false, "MERGEABLE", "already merged"),
        ("CLOSED", false, "MERGEABLE", "closed"),
        ("OPEN", false, "CONFLICTING", "conflicting"),
    ] {
        let mut app = ready_app();
        let View::Review(current) = &mut app.view else {
            panic!()
        };
        current.readiness = Some(Ok(readiness(state, draft, mergeable, "")));
        app.on_key(KeyCode::Char('m'));
        poll(&mut app);
        assert!(app.status.as_deref().unwrap().contains(reason));
        assert!(render(&mut app).contains(reason));
        assert!(review(&app).merge_picker.is_none());
        assert!(app.pending_merge.is_none());
    }
}

#[test]
fn refusals_show_only_the_first_line_and_never_claim_a_merge() {
    for reason in [
        "GraphQL: branch protection requires approval",
        "required status check is pending",
        "GraphQL: permission denied (mergePullRequest)",
    ] {
        let mut app = ready_app();
        let sender = submit(&mut app);
        sender
            .send(Ok(Submission::Refused(format!(
                "{reason}\nprivate second line"
            ))))
            .unwrap();
        poll(&mut app);
        assert!(review(&app)
            .merge_message
            .as_ref()
            .unwrap()
            .contains(reason));
        assert!(!render(&mut app).contains("private second line"));
        assert!(!review(&app).merged);
        assert!(app.pending_merge_check.is_none());
        assert!(app.merge_guards.is_empty());
    }
}

#[test]
fn every_uncertain_write_reconciles_without_retry_including_disconnected_workers() {
    for error in [
        Some("command timed out"),
        Some("connection reset after submission"),
        Some("command output exceeds 32 MB limit"),
        Some("background worker panicked; retry the request"),
        None,
    ] {
        let mut app = ready_app();
        let sender = submit(&mut app);
        if let Some(error) = error {
            sender.send(Err(error.into())).unwrap();
        }
        drop(sender);
        poll(&mut app);
        assert!(render(&mut app).contains("Merge outcome unknown"));
        assert!(!review(&app).merged && !review(&app).can_merge());
        app.on_key(KeyCode::Char('m'));
        poll(&mut app); // No second write can be submitted while verifying.
        assert!(review(&app).merge_picker.is_none());
        verify(&mut app, Ok(readiness("MERGED", false, "UNKNOWN", "")));
        assert!(review(&app).merged);
        assert!(render(&mut app).contains("Merged · merging disabled"));
        assert!(app.pending_merge.is_none() && app.merge_request.is_none());
    }
}

#[test]
fn acknowledgements_and_unknown_results_never_call_open_or_closed_prs_merged() {
    for submission in [Submission::Accepted, Submission::Unknown] {
        for state in ["OPEN", "CLOSED"] {
            let mut app = ready_app();
            let sender = submit(&mut app);
            sender
                .send(Ok(match submission {
                    Submission::Accepted => Submission::Accepted,
                    _ => Submission::Unknown,
                }))
                .unwrap();
            poll(&mut app);
            verify(&mut app, Ok(readiness(state, false, "MERGEABLE", "")));
            assert!(!review(&app).merged && !review(&app).can_merge());
            assert!(!render(&mut app).contains("Merged · merging disabled"));
            assert!(review(&app)
                .merge_message
                .as_ref()
                .unwrap()
                .contains(if state == "OPEN" {
                    "possibly queued"
                } else {
                    "closed"
                }));
            app.on_key(KeyCode::Esc);
            open_loaded_pr(&mut app);
            let sender = status_job(&mut app);
            sender
                .send(Ok(readiness("OPEN", false, "MERGEABLE", "")))
                .unwrap();
            app.poll_readiness_with(|_| panic!());
            app.on_key(KeyCode::Char('m'));
            poll(&mut app);
            assert!(review(&app).merge_picker.is_none());
            assert!(!review(&app).can_merge());
        }
    }
}

#[test]
fn verification_failure_stays_unknown_until_a_manual_read_confirms_merge() {
    let mut app = ready_app();
    let sender = submit(&mut app);
    sender.send(Ok(Submission::Accepted)).unwrap();
    poll(&mut app);
    verify(&mut app, Err("private verification error".into()));
    assert!(!review(&app).merged && !review(&app).can_merge());
    assert!(render(&mut app).contains("Merge outcome unknown"));
    assert!(!render(&mut app).contains("private verification error"));
    app.on_key(KeyCode::Char('r'));
    let (sender, job) = Job::channel();
    app.poll_readiness_with(|_| Ok(job));
    sender
        .send(Ok(readiness("MERGED", false, "UNKNOWN", "")))
        .unwrap();
    app.poll_readiness_with(|_| panic!());
    poll(&mut app);
    assert!(review(&app).merged);
}

#[test]
fn changed_head_during_reconciliation_never_replaces_the_snapshot_or_retries() {
    let mut app = ready_app();
    let sender = submit(&mut app);
    sender.send(Ok(Submission::Unknown)).unwrap();
    poll(&mut app);
    let mut changed = readiness("OPEN", false, "MERGEABLE", "");
    changed.head_ref_oid = "fedcba9876543210".into();
    verify(&mut app, Ok(changed));
    assert!(render(&mut app).contains("PR changed, reload"));
    assert_eq!(
        review(&app).snapshot.as_ref().unwrap().head_sha,
        "abcdef0123456789"
    );
    app.on_key(KeyCode::Char('m'));
    poll(&mut app);
    assert!(review(&app).merge_picker.is_none());
    assert!(!review(&app).merged);
}

#[test]
fn submitted_write_and_verification_survive_navigation_and_keep_the_original_pr_identity() {
    let mut app = ready_app();
    let sender = submit(&mut app);
    app.on_key(KeyCode::Char('j'));
    app.on_key(KeyCode::Char('d'));
    assert!(render(&mut app).contains("Merging…"));
    app.on_key(KeyCode::Char('q')); // Goes back, never cancels the write.
    assert!(matches!(app.view, View::Issues));
    assert!(app.pending_merge.is_some());
    let mut loaded = super::super::super::tests::loaded("");
    loaded.snapshot.url = "https://github.com/o/r/pull/2".into();
    loaded.snapshot.number = 2;
    let (opened, result) = Job::channel();
    app.pending_review = Some(PendingReview {
        title: "Other PR".into(),
        started: Instant::now(),
        result,
    });
    opened.send(Ok(loaded)).unwrap();
    app.poll_review();
    sender.send(Ok(Submission::Unknown)).unwrap();
    poll(&mut app);
    let (sender, job) = Job::channel();
    app.poll_merge_check_with(|url| {
        assert_eq!(url, "https://github.com/o/r/pull/1");
        Ok(job)
    });
    app.on_key(KeyCode::Char('j'));
    app.on_key(KeyCode::Char('u'));
    render(&mut app);
    assert!(app.pending_merge_check.is_some());
    sender
        .send(Ok(readiness("MERGED", false, "UNKNOWN", "")))
        .unwrap();
    app.poll_merge_check_with(|_| panic!("cannot replace running verification"));
    assert!(!review(&app).merged);
    assert!(review(&app).merge_message.is_none());
    assert!(app
        .status
        .as_deref()
        .unwrap()
        .contains("o/r PR #1: Merged (verified)"));
    assert!(app.pending_mutation.is_none());
}

#[test]
fn verification_waits_for_stale_status_worker_and_spawn_failure_never_retries() {
    let mut app = ready_app();
    let sender = submit(&mut app);
    let (stale, job) = Job::channel();
    app.pending_readiness = Some(PendingReadiness {
        generation: app.review_generation - 1,
        result: job,
    });
    sender.send(Ok(Submission::Unknown)).unwrap();
    poll(&mut app);
    app.poll_merge_check_with(|_| panic!("status concurrency is bounded"));
    stale
        .send(Ok(readiness("OPEN", false, "MERGEABLE", "")))
        .unwrap();
    app.poll_readiness_with(|_| panic!("merge check takes priority"));
    app.poll_merge_check_with(|_| Err("could not spawn".into()));
    assert!(app.pending_merge_check.is_none());
    assert!(!review(&app).can_merge());
    assert!(render(&mut app).contains("Merge outcome unknown"));
    poll(&mut app);
    app.poll_merge_check_with(|_| panic!("no automatic retries"));
}
