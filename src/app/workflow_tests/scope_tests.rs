use super::*;

fn tab_bar(app: &mut App) -> String {
    render(app).chars().take(100).collect()
}

#[test]
fn the_app_starts_on_assigned_issues_and_t_toggles_the_issue_scope() {
    let mut app = app();
    assert_eq!(app.scope, IssueScope::Assigned);
    assert!(tab_bar(&mut app).contains(" Assigned work ("));
    app.on_key(KeyCode::Char('t'));
    assert_eq!(app.scope, IssueScope::Team);
    assert!(tab_bar(&mut app).contains(" Team work ("));
    assert!(render(&mut app).contains("No open Team issues"));
    app.on_key(KeyCode::Char('t'));
    assert_eq!(app.scope, IssueScope::Assigned);
    assert!(tab_bar(&mut app).contains(" Assigned work ("));
    // A new App is the next launch: the Issue scope is not saved.
    app.on_key(KeyCode::Char('t'));
    assert_eq!(self::app().scope, IssueScope::Assigned);
}

#[test]
fn the_issue_list_footer_lists_t() {
    let mut app = app();
    let text: String = buffer(&mut app, 60, 30)
        .content
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(text.contains("t scope"));
}

fn errors(app: &App) -> Vec<Option<String>> {
    app.accounts.iter().map(|a| a.list.error.clone()).collect()
}

#[test]
fn the_first_switch_to_team_issues_starts_a_team_fetch_and_later_switches_start_none() {
    let mut app = app();
    // Startup already tried each Assigned fetch; clear that so only `t` is observed.
    for account in &mut app.accounts {
        account.list.error = None;
    }
    app.on_key(KeyCode::Char('t'));
    // Credential-less Accounts record the key error where a Team fetch starts.
    assert_eq!(
        errors(&app),
        [
            Some("work key unavailable".into()),
            Some("personal key unavailable".into())
        ]
    );
    for account in &mut app.accounts {
        account.list.error = None;
    }
    app.on_key(KeyCode::Char('t'));
    assert_eq!(app.scope, IssueScope::Assigned);
    assert_eq!(errors(&app), [None, None]);
    app.on_key(KeyCode::Char('t'));
    assert_eq!(app.scope, IssueScope::Team);
    assert_eq!(errors(&app), [None, None]);
}

#[test]
fn a_later_switch_shows_the_stored_list_immediately_and_keeps_a_running_fetch() {
    let mut app = app();
    app.on_key(KeyCode::Char('t'));
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("team-issue", false)]);
    let (sender, job) = Job::channel();
    app.accounts[1].list.pending = Some(job);
    app.on_key(KeyCode::Char('t'));
    assert!(!render(&mut app).contains("team-issue"));
    app.on_key(KeyCode::Char('t'));
    assert!(render(&mut app).contains("team-issue"));
    sender
        .send(Ok(vec![issue("personal-team", false)]))
        .unwrap();
    wait(&mut app, |a| !a.accounts[1].list.loading());
    assert_eq!(app.accounts[1].list.issues[0].id, "personal-team");
}

#[test]
fn each_scope_keeps_its_own_list_and_selection_per_account() {
    let mut app = app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("a1", false), issue("a2", false)]);
    app.accounts[0].list.selection.select(Some(1));
    app.on_key(KeyCode::Char('t'));
    assert!(app.selected().is_none());
    app.accounts[0].list.apply_snapshot(vec![
        issue("t1", false),
        issue("t2", false),
        issue("t3", false),
    ]);
    app.on_key(KeyCode::Char('G'));
    let text = tab_bar(&mut app);
    assert!(text.contains("work (3)"), "{text}");
    assert_eq!(app.selected().unwrap().1.id, "t3");

    app.on_key(KeyCode::Char('t'));
    let text = render(&mut app);
    assert!(tab_bar(&mut app).contains("work (2)"));
    assert!(text.contains("a1") && !text.contains("t1"));
    assert_eq!(app.selected().unwrap().1.id, "a2");

    app.on_key(KeyCode::Char('t'));
    assert_eq!(app.selected().unwrap().1.id, "t3");
}

#[test]
fn the_all_accounts_queue_follows_the_scope_and_keeps_its_selection_per_scope() {
    let mut app = app();
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("work-a", false)]);
    app.accounts[1]
        .list
        .apply_snapshot(vec![issue("personal-a", true)]);
    app.on_key(KeyCode::Char('a'));
    app.poll_background();
    app.on_key(KeyCode::Char('j'));
    assert_eq!(app.selected().unwrap().1.id, "work-a");

    app.on_key(KeyCode::Char('t'));
    assert!(app.all_accounts_queue());
    assert!(app.selected().is_none());
    assert_eq!(app.queue.len(), 0);
    app.accounts[0]
        .list
        .apply_snapshot(vec![issue("work-t", false)]);
    app.accounts[1]
        .list
        .apply_snapshot(vec![issue("personal-t", false), issue("personal-u", true)]);
    app.poll_background();
    assert_eq!(app.queue.len(), 3);
    // In Review first, then Account order: personal-u, work-t, personal-t.
    app.on_key(KeyCode::Char('j'));
    app.on_key(KeyCode::Char('j'));
    let selected = |app: &App| app.selected().map(|(a, i)| (a, i.id.clone()));
    assert_eq!(selected(&app), Some((1, "personal-t".into())));

    app.on_key(KeyCode::Char('t'));
    assert_eq!(selected(&app), Some((0, "work-a".into())));
    assert_eq!(app.queue.len(), 2);
    app.on_key(KeyCode::Char('t'));
    assert_eq!(selected(&app), Some((1, "personal-t".into())));
}

#[test]
fn t_does_nothing_in_the_pr_view_pickers_or_while_a_loading_box_shows() {
    let mut app = app();
    open_loaded_pr(&mut app);
    assert!(matches!(app.view, View::Review(_)));
    app.on_key(KeyCode::Char('t'));
    assert_eq!(app.scope, IssueScope::Assigned);

    picker(&mut app);
    app.on_key(KeyCode::Char('t'));
    assert!(matches!(app.view, View::StatePicker(_)));
    assert_eq!(app.scope, IssueScope::Assigned);

    app.view = View::PrPicker(PrPicker {
        title: "HL-1".into(),
        options: Vec::new(),
        list: ListState::default(),
    });
    app.on_key(KeyCode::Char('t'));
    assert!(matches!(app.view, View::PrPicker(_)));
    assert_eq!(app.scope, IssueScope::Assigned);

    app.view = View::Issues;
    let (_sender, result) = Job::channel();
    app.pending_review = Some(PendingReview {
        title: "HL-1 Test PR".into(),
        started: Instant::now(),
        result,
    });
    app.on_key(KeyCode::Char('t'));
    assert!(app.loading_review().is_some());
    assert_eq!(app.scope, IssueScope::Assigned);
}
