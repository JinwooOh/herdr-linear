use super::*;

fn hidden(app: &App, account: usize) -> &IssueList {
    app.accounts[account]
        .hidden
        .as_ref()
        .expect("the other Issue scope was shown")
}

fn ids(list: &IssueList) -> Vec<&str> {
    list.issues.iter().map(|i| i.id.as_str()).collect()
}

/// Shows Team issues on every Account, then clears every list's error, so the key error a
/// credential-less Account records marks exactly the lists that start a fetch afterwards.
fn team_app() -> App {
    let mut app = app();
    app.on_key(KeyCode::Char('t'));
    for account in &mut app.accounts {
        account.list.error = None;
        account.hidden.as_mut().unwrap().error = None;
    }
    app
}

/// For each Account: whether its (Team, Assigned) list started a fetch.
fn fetched(app: &App) -> Vec<(bool, bool)> {
    (0..app.accounts.len())
        .map(|a| {
            (
                app.accounts[a].list.error.is_some(),
                hidden(app, a).error.is_some(),
            )
        })
        .collect()
}

#[test]
fn polling_refreshes_only_each_accounts_current_scope() {
    let mut app = team_app();
    let due = Instant::now() - REFRESH_INTERVAL;
    for account in &mut app.accounts {
        account.list.refreshed = due;
        account.hidden.as_mut().unwrap().refreshed = due;
    }
    app.poll_background();
    assert_eq!(fetched(&app), [(true, false), (true, false)]);
    // The hidden scope stays due but still starts nothing on later polls.
    app.poll_background();
    assert_eq!(fetched(&app), [(true, false), (true, false)]);
}

#[test]
fn r_refreshes_the_current_scope_of_the_active_account_or_every_account_in_the_queue() {
    let mut app = team_app();
    app.on_key(KeyCode::Char('2'));
    app.on_key(KeyCode::Char('r'));
    assert_eq!(fetched(&app), [(false, false), (true, false)]);

    let mut app = team_app();
    app.on_key(KeyCode::Char('a'));
    app.on_key(KeyCode::Char('r'));
    assert_eq!(fetched(&app), [(true, false), (true, false)]);
}

#[test]
fn a_hidden_scope_fetch_that_finishes_after_switching_updates_that_scope_only() {
    let mut app = app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("assigned-old", false)]);
    let (work, job) = Job::channel();
    app.accounts[0].list.pending = Some(job);
    let (personal, job) = Job::channel();
    app.accounts[1].list.pending = Some(job);
    app.on_key(KeyCode::Char('t'));
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("team-a", false)]);
    for account in &mut app.accounts {
        account.list.error = None;
    }

    work.send(Ok(vec![issue("assigned-new", false)])).unwrap();
    personal.send(Err("offline".into())).unwrap();
    app.poll_background();

    assert!(!hidden(&app, 0).loading() && !hidden(&app, 1).loading());
    assert_eq!(ids(hidden(&app, 0)), ["assigned-new"]);
    assert_eq!(hidden(&app, 1).error.as_deref(), Some("offline"));
    assert_eq!(ids(&app.accounts[0].list), ["team-a"]);
    assert_eq!(app.accounts[1].list.error, None);
    let text = render(&mut app);
    assert!(text.contains("team-a") && !text.contains("assigned-new"));

    app.on_key(KeyCode::Char('t'));
    let text = render(&mut app);
    assert!(text.contains("assigned-new") && !text.contains("assigned-old"));
}

#[test]
fn the_all_accounts_queue_restores_its_selection_by_account_and_issue_after_hidden_results_land() {
    let mut app = app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("work-a", false)]);
    app.accounts[1]
        .list
        .apply_snapshot(vec![issue("personal-b", false)]);
    let (work, job) = Job::channel();
    app.accounts[0].list.pending = Some(job);
    app.on_key(KeyCode::Char('a'));
    app.poll_background();
    // Account order breaks the tie: work-a, then personal-b.
    app.on_key(KeyCode::Char('j'));
    let selected = |app: &App| app.selected().map(|(a, i)| (a, i.id.clone()));
    assert_eq!(selected(&app), Some((1, "personal-b".into())));

    app.on_key(KeyCode::Char('t'));
    // An In Review issue sorts first and moves personal-b down a row.
    work.send(Ok(vec![issue("work-a", false), issue("work-new", true)]))
        .unwrap();
    app.poll_background();
    app.on_key(KeyCode::Char('t'));
    assert_eq!(selected(&app), Some((1, "personal-b".into())));
}

#[test]
fn a_team_scope_failure_stays_in_its_account_and_keeps_the_last_list() {
    let mut app = team_app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("work-team", false)]);
    app.accounts[1]
        .list
        .apply_snapshot(vec![issue("personal-team", false)]);
    let (work, job) = Job::channel();
    app.accounts[0].list.pending = Some(job);
    work.send(Err("Team issues: offline".into())).unwrap();
    app.poll_background();

    assert_eq!(ids(&app.accounts[0].list), ["work-team"]);
    assert_eq!(
        app.accounts[0].list.error.as_deref(),
        Some("Team issues: offline")
    );
    assert_eq!(ids(&app.accounts[1].list), ["personal-team"]);
    assert_eq!(fetched(&app), [(true, false), (false, false)]);
    let text = render(&mut app);
    assert!(text.contains("work-team") && text.contains("work: Team issues: offline"));
    // Only the failing Account tab is marked; the other keeps its count and no marker.
    assert!(text.contains("work (1) !") && text.contains("personal (1) |"));
    assert!(!text.contains("personal:"));
}

fn done() -> linear::State {
    linear::State {
        id: "done".into(),
        name: "Done".into(),
        kind: "completed".into(),
        position: 0.0,
    }
}

#[test]
fn a_state_change_finishing_after_t_routes_to_its_account_and_refreshes_the_current_scope() {
    let mut app = app();
    for account in &mut app.accounts {
        account
            .list
            .apply_snapshot(vec![issue("same", false), issue("other", false)]);
    }
    let (sender, job) = Job::channel();
    app.pending_mutation = Some(PendingMutation {
        account: 1,
        issue_id: "same".into(),
        title: "HL-1".into(),
        job,
    });
    app.on_key(KeyCode::Char('t'));
    for account in &mut app.accounts {
        account.list.apply_snapshot(vec![issue("same", false)]);
        account.list.error = None;
        account.hidden.as_mut().unwrap().error = None;
    }
    // A Team fetch started before the write finished must not undo the change.
    let (stale, job) = Job::channel();
    app.accounts[1].list.pending = Some(job);

    sender.send(Ok(done())).unwrap();
    app.poll_background();
    let _ = stale.send(Ok(vec![issue("same", false)]));
    app.poll_background();

    assert!(app.status.as_ref().unwrap().contains("personal"));
    assert!(app.accounts[1].list.issues.is_empty());
    assert_eq!(ids(hidden(&app, 1)), ["other"]);
    assert_eq!(ids(&app.accounts[0].list), ["same"]);
    assert_eq!(ids(hidden(&app, 0)), ["other", "same"]);
    // Only the mutated Account refreshes, and only its current scope.
    assert_eq!(fetched(&app), [(false, false), (true, false)]);
}
