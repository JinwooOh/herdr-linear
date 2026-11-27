#[cfg(test)]
use std::sync::mpsc;
use std::time::{Duration, Instant};

use anyhow::Result;
use ratatui::{
    crossterm::event::{self, Event, KeyCode, KeyEventKind},
    widgets::ListState,
    DefaultTerminal,
};

use crate::{
    config::{AccountConfig, MergeOptions},
    linear,
    merge::{self, MergeMethod},
    review::{self, MergePicker, PrSnapshot, Review},
    ui,
    worker::Job,
};

const HALF_PAGE: i32 = 15;
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

pub enum View {
    Issues,
    Review(Review),
    PrPicker(PrPicker),
    StatePicker(StatePicker),
}

pub struct PrPicker {
    pub title: String,
    pub options: Vec<linear::Attachment>,
    pub list: ListState,
}

pub struct StatePicker {
    pub account: usize,
    pub issue_id: String,
    pub title: String,
    pub options: Vec<linear::State>,
    pub current_state: String,
    pub started: Instant,
    pub confirming: bool,
    pub list: ListState,
    pending: Option<Job<Vec<linear::State>>>,
    pub error: Option<String>,
}

/// Which open issues the app shows. One Issue scope applies to every Account tab and
/// the All-accounts queue. It always starts as Assigned issues and is never saved.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum IssueScope {
    #[default]
    Assigned,
    Team,
}

impl IssueScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Assigned => "Assigned",
            Self::Team => "Team",
        }
    }

    fn toggled(self) -> Self {
        match self {
            Self::Assigned => Self::Team,
            Self::Team => Self::Assigned,
        }
    }

    fn fetch(self) -> issue_list::Fetch {
        match self {
            Self::Assigned => linear::fetch_issues,
            Self::Team => linear::fetch_team_issues,
        }
    }
}

pub struct Account {
    pub name: String,
    key: Result<String, String>,
    /// The issue list of the App's current Issue scope.
    pub list: IssueList,
    /// The other Issue scope's list, kept as it was; `None` until that scope is first shown.
    hidden: Option<IssueList>,
}

impl Account {
    fn new(config: AccountConfig) -> Self {
        let mut account = Self {
            name: config.name,
            key: config.key,
            list: IssueList::default(),
            hidden: None,
        };
        account.refresh(IssueScope::default());
        account
    }

    /// Refreshes `list`, which must be the list of `scope`.
    fn refresh(&mut self, scope: IssueScope) {
        self.list.refresh(&self.key, scope.fetch());
    }

    /// Makes `scope`'s list current. A scope shown for the first time starts its fetch.
    fn show(&mut self, scope: IssueScope) {
        let list = match self.hidden.take() {
            Some(list) => list,
            None => {
                let mut list = IssueList::default();
                list.refresh(&self.key, scope.fetch());
                list
            }
        };
        self.hidden = Some(std::mem::replace(&mut self.list, list));
    }
}

mod issue_list;
pub use issue_list::IssueList;

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

struct PendingReview {
    title: String,
    started: Instant,
    result: Job<review::LoadedReview>,
}

struct PendingReadiness {
    generation: u64,
    result: Job<review::Readiness>,
}

struct PendingMethods {
    generation: u64,
    snapshot: PrSnapshot,
    job: Job<Vec<MergeMethod>>,
}

struct PendingMerge {
    snapshot: PrSnapshot,
    job: Job<merge::Submission>,
}

struct PendingMergeCheck {
    snapshot: PrSnapshot,
    submission: merge::Submission,
    job: Option<Job<review::Readiness>>,
}

struct PendingMutation {
    account: usize,
    issue_id: String,
    title: String,
    job: Job<linear::State>,
}

pub struct App {
    pub accounts: Vec<Account>,
    /// accounts.len() selects the All-accounts queue instead of an Account tab.
    pub active: usize,
    pub scope: IssueScope,
    pub status: Option<String>,
    pub view: View,
    pub queue: Vec<(usize, usize)>,
    pub queue_list: ListState,
    /// The All-accounts queue selection of the Issue scope not shown.
    hidden_queue_list: ListState,
    quit: bool,
    pending_review: Option<PendingReview>,
    // Retained even after leaving the PR view: at most one status worker can run.
    pending_readiness: Option<PendingReadiness>,
    review_generation: u64,
    pending_mutation: Option<PendingMutation>,
    pub merge_options: MergeOptions,
    pending_methods: Option<PendingMethods>,
    merge_request: Option<(PrSnapshot, MergeMethod)>,
    pending_merge: Option<PendingMerge>,
    pending_merge_check: Option<PendingMergeCheck>,
    // Acknowledged/uncertain writes must not be resubmitted, even after reopening.
    merge_guards: std::collections::HashSet<String>,
}

impl App {
    pub fn new(configs: Vec<AccountConfig>) -> Self {
        Self {
            accounts: configs.into_iter().map(Account::new).collect(),
            active: 0,
            scope: IssueScope::default(),
            status: None,
            view: View::Issues,
            queue: Vec::new(),
            queue_list: ListState::default(),
            hidden_queue_list: ListState::default(),
            quit: false,
            pending_review: None,
            pending_readiness: None,
            review_generation: 0,
            pending_mutation: None,
            merge_options: MergeOptions::default(),
            pending_methods: None,
            merge_request: None,
            pending_merge: None,
            pending_merge_check: None,
            merge_guards: std::collections::HashSet::new(),
        }
    }

    pub fn all_accounts_queue(&self) -> bool {
        self.active == self.accounts.len()
    }

    pub fn selected(&self) -> Option<(usize, &linear::Issue)> {
        if self.all_accounts_queue() {
            let &(account, index) = self.queue_list.selected().and_then(|i| self.queue.get(i))?;
            Some((account, self.accounts.get(account)?.list.issues.get(index)?))
        } else {
            self.accounts
                .get(self.active)?
                .list
                .selected()
                .map(|issue| (self.active, issue))
        }
    }

    fn list_mut(&mut self) -> &mut ListState {
        if self.all_accounts_queue() {
            &mut self.queue_list
        } else {
            &mut self.accounts[self.active].list.selection
        }
    }

    fn rebuild_queue(&mut self, selected: Option<(usize, String)>) {
        self.queue = self
            .accounts
            .iter()
            .enumerate()
            .flat_map(|(a, account)| (0..account.list.issues.len()).map(move |i| (a, i)))
            .collect();
        self.queue.sort_by(|&(a, i), &(b, j)| {
            let x = &self.accounts[a].list.issues[i];
            let y = &self.accounts[b].list.issues[j];
            y.is_in_review()
                .cmp(&x.is_in_review())
                .then(y.updated_at.cmp(&x.updated_at))
                .then(a.cmp(&b))
                .then(x.id.cmp(&y.id))
        });
        let index = selected
            .and_then(|(a, id)| {
                self.queue
                    .iter()
                    .position(|&(b, i)| a == b && self.accounts[b].list.issues[i].id == id)
            })
            .unwrap_or(
                self.queue_list
                    .selected()
                    .unwrap_or(0)
                    .min(self.queue.len().saturating_sub(1)),
            );
        self.queue_list.select(if self.queue.is_empty() {
            None
        } else {
            Some(index)
        });
    }

    fn poll_background(&mut self) {
        let selected = self
            .queue_list
            .selected()
            .and_then(|i| self.queue.get(i))
            .map(|&(a, i)| (a, self.accounts[a].list.issues[i].id.clone()));
        for (index, account) in self.accounts.iter_mut().enumerate() {
            account.list.poll();
            if !self
                .pending_mutation
                .as_ref()
                .is_some_and(|m| m.account == index)
                && account.list.refreshed.elapsed() >= REFRESH_INTERVAL
            {
                account.refresh(self.scope);
            }
        }
        self.poll_mutation();
        self.rebuild_queue(selected);
        self.poll_review();
        self.poll_merge();
        self.poll_merge_check_with(|url| Job::spawn(move || review::load_readiness(&url)));
        self.poll_readiness();
        if let View::Review(review) = &mut self.view {
            review.poll_file();
        }
        if let View::StatePicker(picker) = &mut self.view {
            if let Some(result) = picker.pending.as_ref().and_then(Job::poll) {
                picker.pending = None;
                match result {
                    Ok(states) => {
                        if states.is_empty() {
                            picker.error = Some("This team has no selectable states.".into());
                        }
                        picker.options = states;
                        picker.list.select(if picker.options.is_empty() {
                            None
                        } else {
                            picker
                                .options
                                .iter()
                                .position(|s| s.id == picker.current_state)
                        });
                    }
                    Err(error) => picker.error = Some(error),
                }
            }
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> Result<()> {
        while !self.quit {
            self.poll_background();
            terminal.draw(|frame| {
                ui::draw(frame, self);
                crate::display::sanitize_buffer(frame.buffer_mut());
            })?;
            if event::poll(Duration::from_millis(100))? {
                if let Event::Key(key) = event::read()? {
                    self.on_event(key);
                }
            }
        }
        Ok(())
    }

    fn on_event(&mut self, key: ratatui::crossterm::event::KeyEvent) {
        use ratatui::crossterm::event::KeyModifiers;
        if key.kind != KeyEventKind::Press {
            return;
        }
        // Raw mode delivers Ctrl+C as a key. Keep supervising submitted writes.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.request_quit();
        } else {
            self.on_key(key.code);
        }
    }

    fn request_quit(&mut self) {
        if self.merge_request.is_some()
            || self.pending_merge.is_some()
            || self.pending_merge_check.is_some()
        {
            self.status =
                Some("Merge submitted; wait for outcome verification before quitting.".into());
        } else {
            self.quit = true;
        }
    }

    fn on_key(&mut self, code: KeyCode) {
        self.status = None;
        // The loading box is modal: only cancel and quit act, so a stray key
        // cannot silently abandon a slow load.
        if self.pending_review.is_some() {
            match code {
                KeyCode::Esc => self.cancel_review(),
                KeyCode::Char('q') => self.request_quit(),
                _ => {}
            }
            return;
        }
        if matches!(self.view, View::PrPicker(_) | View::StatePicker(_)) {
            self.picker_key(code);
            return;
        }
        if matches!(&self.view, View::Review(review) if review.merge_picker.is_some()) {
            self.merge_picker_key(code);
            return;
        }
        if code == KeyCode::Char('m') && matches!(self.view, View::Review(_)) {
            self.open_merge_picker();
            return;
        }
        if let View::Review(review) = &mut self.view {
            match code {
                KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => self.view = View::Issues,
                KeyCode::Char('j') | KeyCode::Down => review.select_next(),
                KeyCode::Char('k') | KeyCode::Up => review.select_previous(),
                KeyCode::Char('d') | KeyCode::PageDown => review.scroll_by(HALF_PAGE),
                KeyCode::Char('u') | KeyCode::PageUp => review.scroll_by(-HALF_PAGE),
                KeyCode::Char('h') | KeyCode::Left => review.scroll_horizontal(-8),
                KeyCode::Char('l') | KeyCode::Right => review.scroll_horizontal(8),
                KeyCode::Char('f') => review.toggle_full_file(),
                KeyCode::Char('r') if !review.readiness_loading => review.readiness_started = None,
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.request_quit(),
            KeyCode::Char('j') | KeyCode::Down => self.list_mut().select_next(),
            KeyCode::Char('k') | KeyCode::Up => self.list_mut().select_previous(),
            KeyCode::Char('g') | KeyCode::Home => self.list_mut().select_first(),
            KeyCode::Char('G') | KeyCode::End => self.list_mut().select_last(),
            KeyCode::Char('r') => {
                let all = self.all_accounts_queue();
                for (i, account) in self.accounts.iter_mut().enumerate() {
                    if (all || i == self.active)
                        && !self
                            .pending_mutation
                            .as_ref()
                            .is_some_and(|m| m.account == i)
                    {
                        account.refresh(self.scope);
                    }
                }
            }
            KeyCode::Tab => self.active = (self.active + 1) % (self.accounts.len() + 1),
            KeyCode::BackTab => {
                self.active = (self.active + self.accounts.len()) % (self.accounts.len() + 1)
            }
            KeyCode::Char('a') => self.active = self.accounts.len(),
            KeyCode::Char(c @ '1'..='9') => {
                let index = c as usize - '1' as usize;
                if index < self.accounts.len() {
                    self.active = index;
                }
            }
            KeyCode::Enter => self.open_review(),
            KeyCode::Char('s') => self.open_states(),
            KeyCode::Char('t') => self.toggle_scope(),
            _ => {}
        }
    }

    fn toggle_scope(&mut self) {
        self.scope = self.scope.toggled();
        for account in &mut self.accounts {
            account.show(self.scope);
        }
        // A hidden list does not change, so the stored queue index still names the same issue.
        std::mem::swap(&mut self.queue_list, &mut self.hidden_queue_list);
        self.rebuild_queue(None);
    }

    fn picker_key(&mut self, code: KeyCode) {
        if let View::StatePicker(p) = &mut self.view {
            if p.confirming {
                if code == KeyCode::Char('y') {
                    let old = std::mem::replace(&mut self.view, View::Issues);
                    if let View::StatePicker(p) = old {
                        if let Some(state) = p.list.selected().and_then(|i| p.options.get(i)) {
                            self.start_mutation(p.account, p.issue_id, p.title, state.id.clone());
                        }
                    }
                } else {
                    p.confirming = false;
                }
                return;
            }
        }
        if matches!(code, KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace) {
            self.view = View::Issues;
            return;
        }
        let (list, len) = match &mut self.view {
            View::PrPicker(p) => (&mut p.list, p.options.len()),
            View::StatePicker(p) => (&mut p.list, p.options.len()),
            _ => return,
        };
        match code {
            KeyCode::Char('j') | KeyCode::Down if len > 0 => {
                list.select(Some((list.selected().unwrap_or(0) + 1).min(len - 1)))
            }
            KeyCode::Char('k') | KeyCode::Up if len > 0 => {
                list.select(Some(list.selected().unwrap_or(0).saturating_sub(1)))
            }
            KeyCode::Enter => {
                let old = std::mem::replace(&mut self.view, View::Issues);
                match old {
                    View::PrPicker(p) => {
                        if let Some(pr) = p.list.selected().and_then(|i| p.options.get(i)) {
                            self.open_pr(p.title, pr.clone());
                        }
                    }
                    View::StatePicker(mut p) => {
                        if let Some(state) = p.list.selected().and_then(|i| p.options.get(i)) {
                            if state.id == p.current_state {
                                return;
                            }
                            if matches!(state.kind.as_str(), "completed" | "canceled" | "duplicate")
                            {
                                p.confirming = true;
                                self.view = View::StatePicker(p);
                            } else {
                                self.start_mutation(
                                    p.account,
                                    p.issue_id,
                                    p.title,
                                    state.id.clone(),
                                );
                            }
                        } else {
                            self.view = View::StatePicker(p);
                        }
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    fn open_review(&mut self) {
        if self.pending_review.is_some() {
            return;
        }
        let Some((_, issue)) = self.selected() else {
            return;
        };
        let title = issue.identifier.clone();
        let prs: Vec<_> = issue.pull_requests().cloned().collect();
        match prs.len() {
            0 => self.status = Some(format!("{title} has no linked pull request")),
            1 => self.open_pr(title, prs[0].clone()),
            _ => {
                let mut list = ListState::default();
                list.select(Some(0));
                self.view = View::PrPicker(PrPicker {
                    title,
                    options: prs,
                    list,
                });
            }
        }
    }

    fn open_pr(&mut self, issue: String, pr: linear::Attachment) {
        let title = format!("{issue}  {}", pr.title.as_deref().unwrap_or(&pr.url));
        self.start_review(title, move || {
            review::load(&pr.url).map_err(|e| format!("{e:#}"))
        });
    }

    fn start_review(
        &mut self,
        title: String,
        load: impl FnOnce() -> Result<review::LoadedReview, String> + Send + 'static,
    ) {
        match Job::spawn(move || load().map_err(anyhow::Error::msg)) {
            Ok(result) => {
                self.status = None;
                self.pending_review = Some(PendingReview {
                    title,
                    started: Instant::now(),
                    result,
                });
            }
            Err(error) => self.status = Some(error),
        }
    }

    pub fn loading_since(&self) -> Option<Instant> {
        self.pending_review.as_ref().map(|p| p.started)
    }

    pub fn loading_review(&self) -> Option<&str> {
        self.pending_review.as_ref().map(|p| p.title.as_str())
    }
    fn cancel_review(&mut self) {
        self.pending_review = None;
    }

    fn poll_review(&mut self) {
        let Some(pending) = &self.pending_review else {
            return;
        };
        let Some(result) = pending.result.poll() else {
            return;
        };
        let pending = self.pending_review.take().expect("pending review exists");
        match result {
            Ok(loaded) => {
                self.status = None;
                self.review_generation += 1;
                let mut review = Review::from_loaded(pending.title, loaded);
                review.merge_unresolved = review
                    .snapshot
                    .as_ref()
                    .is_some_and(|snapshot| self.merge_guards.contains(&snapshot.url));
                if review.merge_unresolved {
                    review.merge_message = Some(
                        "Merge unresolved or queued; refresh PR status, do not resubmit.".into(),
                    );
                }
                self.view = View::Review(review);
            }
            Err(error) => self.status = Some(error),
        }
    }

    fn poll_readiness(&mut self) {
        self.poll_readiness_with(|url| Job::spawn(move || review::load_readiness(&url)));
    }

    fn poll_readiness_with(
        &mut self,
        spawn: impl FnOnce(String) -> Result<Job<review::Readiness>, String>,
    ) {
        if let Some(result) = self
            .pending_readiness
            .as_ref()
            .and_then(|p| p.result.poll())
        {
            let pending = self
                .pending_readiness
                .take()
                .expect("pending status exists");
            if let View::Review(review) = &mut self.view {
                if pending.generation == self.review_generation {
                    review.readiness_loading = false;
                    review.readiness = Some(result.and_then(|status| {
                        if review.merge_unresolved && status.state == "MERGED" {
                            review.merged = true;
                            review.merge_message = None;
                            return Ok(status);
                        }
                        if review
                            .snapshot
                            .as_ref()
                            .is_some_and(|s| s.head_sha == status.head_ref_oid)
                        {
                            Ok(status)
                        } else {
                            Err("PR head changed; reopen PR to reload diff".into())
                        }
                    }));
                }
            }
        }
        if self.pending_readiness.is_some() {
            return;
        }
        let View::Review(review) = &mut self.view else {
            return;
        };
        let Some(snapshot) = &review.snapshot else {
            return;
        };
        // Do not refresh underneath a merge confirmation or overwrite a known success.
        if review.merged
            || review.merge_picker.is_some()
            || self.pending_merge.is_some()
            || self.pending_merge_check.is_some()
            || self.merge_request.is_some()
        {
            return;
        }
        if review
            .readiness_started
            .is_some_and(|started| started.elapsed() < REFRESH_INTERVAL)
        {
            return;
        }
        review.readiness_started = Some(Instant::now());
        let url = snapshot.original_url.clone();
        match spawn(url) {
            Ok(result) => {
                review.readiness_loading = true;
                self.pending_readiness = Some(PendingReadiness {
                    generation: self.review_generation,
                    result,
                });
            }
            Err(error) => review.readiness = Some(Err(error)),
        }
    }

    fn open_merge_picker(&mut self) {
        if !self.merge_options.enable_merge
            || self.pending_merge.is_some()
            || self.pending_merge_check.is_some()
            || self.merge_request.is_some()
        {
            return;
        }
        let View::Review(review) = &mut self.view else {
            return;
        };
        if let Some(reason) = review.merge_unavailable() {
            self.status = Some(format!("Merge unavailable: {reason}"));
            return;
        }
        review.merge_picker = Some(MergePicker {
            options: Vec::new(),
            list: ListState::default(),
            confirming: false,
            loading: true,
            error: None,
        });
    }

    fn merge_picker_key(&mut self, code: KeyCode) {
        let View::Review(review) = &mut self.view else {
            return;
        };
        let Some(picker) = &mut review.merge_picker else {
            return;
        };
        if picker.confirming {
            picker.confirming = false;
            if code == KeyCode::Char('y') {
                let method = picker
                    .list
                    .selected()
                    .and_then(|i| picker.options.get(i))
                    .copied();
                if self.merge_options.enable_merge
                    && self.pending_merge.is_none()
                    && self.pending_merge_check.is_none()
                    && self.merge_request.is_none()
                    && review.can_merge()
                {
                    if let (Some(snapshot), Some(method)) = (&review.snapshot, method) {
                        self.merge_guards.insert(snapshot.url.clone());
                        self.merge_request = Some((snapshot.as_ref().clone(), method));
                        review.merge_unresolved = true;
                        // Any pre-write status result is now stale, but retain its job
                        // until completion to keep worker concurrency bounded.
                        self.review_generation += 1;
                        review.readiness_loading = false;
                        review.merge_message =
                            Some("Merging… (submitted changes cannot be cancelled)".into());
                    }
                }
                review.merge_picker = None;
            }
            // Even Esc only returns to the picker from confirmation.
            return;
        }
        let len = picker.options.len();
        match code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => review.merge_picker = None,
            KeyCode::Char('j') | KeyCode::Down if len > 0 => {
                picker
                    .list
                    .select(Some((picker.list.selected().unwrap_or(0) + 1).min(len - 1)));
            }
            KeyCode::Char('k') | KeyCode::Up if len > 0 => {
                picker
                    .list
                    .select(Some(picker.list.selected().unwrap_or(0).saturating_sub(1)));
            }
            KeyCode::Enter if picker.list.selected().is_some() => picker.confirming = true,
            _ => {}
        }
    }

    fn poll_merge(&mut self) {
        self.poll_merge_with(
            |snapshot| Job::spawn(move || merge::load_methods(&snapshot)),
            |snapshot, method| Job::spawn(move || merge::submit(&snapshot, method)),
        );
    }

    /// Jobs are injected at the App workflow seam; tests never invoke a write API.
    fn poll_merge_with(
        &mut self,
        methods: impl FnOnce(PrSnapshot) -> Result<Job<Vec<MergeMethod>>, String>,
        submit: impl FnOnce(PrSnapshot, MergeMethod) -> Result<Job<merge::Submission>, String>,
    ) {
        if let Some(result) = self.pending_merge.as_ref().and_then(|p| p.job.poll()) {
            let pending = self.pending_merge.take().expect("pending merge exists");
            // Worker panic/disconnect and every unclassified error are uncertain,
            // not just literal timeouts. Never retry a write automatically.
            let submission = result.unwrap_or(merge::Submission::Unknown);
            let message = match &submission {
                merge::Submission::Changed => {
                    "PR changed, reload: reopen PR to reload diff.".into()
                }
                merge::Submission::Refused(reason) => format!(
                    "Merge refused: {}",
                    crate::display::safe(
                        reason.lines().next().unwrap_or("GitHub refused the merge")
                    )
                ),
                merge::Submission::Accepted => "Merge request accepted; verifying PR state…".into(),
                merge::Submission::Unknown => "Merge outcome unknown; re-fetching PR state…".into(),
            };
            let reconcile = matches!(
                submission,
                merge::Submission::Accepted | merge::Submission::Unknown
            );
            if !reconcile {
                self.merge_guards.remove(&pending.snapshot.url);
            }
            self.merge_message_for(&pending.snapshot, message, reconcile);
            if reconcile {
                self.pending_merge_check = Some(PendingMergeCheck {
                    snapshot: pending.snapshot,
                    submission,
                    job: None,
                });
            }
        }
        if let Some(result) = self.pending_methods.as_ref().and_then(|p| p.job.poll()) {
            let pending = self.pending_methods.take().expect("pending methods exist");
            if let View::Review(review) = &mut self.view {
                if pending.generation == self.review_generation
                    && review.snapshot.as_deref() == Some(&pending.snapshot)
                {
                    if let Some(picker) = &mut review.merge_picker {
                        picker.loading = false;
                        match result {
                            Ok(options) => {
                                let selected = options
                                    .iter()
                                    .position(|m| *m == self.merge_options.merge_method)
                                    .or_else(|| {
                                        options.iter().position(|m| *m == MergeMethod::Squash)
                                    })
                                    .or_else(|| (!options.is_empty()).then_some(0));
                                picker.list.select(selected);
                                if options.is_empty() {
                                    picker.error =
                                        Some("Repository allows no merge methods.".into());
                                }
                                picker.options = options;
                            }
                            Err(_) => {
                                picker.error = Some(
                                    "Could not load repository merge methods; cancel and retry."
                                        .into(),
                                )
                            }
                        }
                    }
                }
            }
        }
        if let Some((snapshot, method)) = self.merge_request.take() {
            // A queued request contains the confirmed identity, never current selection.
            match submit(snapshot.clone(), method) {
                Ok(job) => self.pending_merge = Some(PendingMerge { snapshot, job }),
                Err(_) => {
                    self.merge_guards.remove(&snapshot.url);
                    self.merge_message_for(
                        &snapshot,
                        "Could not start merge worker; no merge submitted.".into(),
                        false,
                    );
                }
            }
        }
        if self.pending_methods.is_some() {
            return;
        }
        let View::Review(review) = &mut self.view else {
            return;
        };
        let Some(picker) = &mut review.merge_picker else {
            return;
        };
        if !picker.loading {
            return;
        }
        let Some(snapshot) = review.snapshot.as_deref().cloned() else {
            return;
        };
        match methods(snapshot.clone()) {
            Ok(job) => {
                self.pending_methods = Some(PendingMethods {
                    generation: self.review_generation,
                    snapshot,
                    job,
                })
            }
            Err(_) => {
                picker.loading = false;
                picker.error =
                    Some("Could not start repository methods worker; cancel and retry.".into());
            }
        }
    }

    /// Report the captured PR identity even if the user has left its PR view.
    fn merge_message_for(&mut self, snapshot: &PrSnapshot, message: String, unresolved: bool) {
        self.status = Some(format!(
            "{} PR #{}: {message}",
            snapshot.repository, snapshot.number
        ));
        if let View::Review(review) = &mut self.view {
            if review
                .snapshot
                .as_ref()
                .is_some_and(|s| s.url == snapshot.url)
            {
                review.readiness = if message.starts_with("PR changed, reload") {
                    Some(Err("PR head changed; reopen PR to reload diff".into()))
                } else {
                    None
                };
                review.merge_message = Some(message);
                review.merge_unresolved = unresolved;
                review.readiness_loading = false;
                review.readiness_started = None;
            }
        }
    }

    /// Reconcile even after navigation. A gh acknowledgement may mean queued,
    /// and a lost response may mean merged. Only a fresh MERGED state is proof.
    fn poll_merge_check_with(
        &mut self,
        spawn: impl FnOnce(String) -> Result<Job<review::Readiness>, String>,
    ) {
        let Some(pending) = &self.pending_merge_check else {
            return;
        };
        if let Some(job) = &pending.job {
            if let Some(result) = job.poll() {
                let pending = self.pending_merge_check.take().expect("merge check exists");
                self.apply_merge_check(pending, result);
            }
            return;
        }
        // Retain any invalidated status worker until it finishes; at most one
        // status subprocess runs at a time.
        if self.pending_readiness.is_some() {
            return;
        }
        let url = pending.snapshot.url.clone();
        match spawn(url) {
            Ok(job) => {
                self.pending_merge_check
                    .as_mut()
                    .expect("merge check exists")
                    .job = Some(job)
            }
            Err(error) => {
                let pending = self.pending_merge_check.take().expect("merge check exists");
                self.apply_merge_check(pending, Err(error));
            }
        }
    }

    fn apply_merge_check(
        &mut self,
        pending: PendingMergeCheck,
        result: Result<review::Readiness, String>,
    ) {
        let merged = result.as_ref().is_ok_and(|status| status.state == "MERGED");
        let message = if merged {
            "Merged (verified)".into()
        } else {
            let prefix = if result.is_err() || pending.submission == merge::Submission::Unknown {
                "Merge outcome unknown"
            } else {
                "Merge not confirmed"
            };
            let detail = match &result {
                Ok(status) if status.head_ref_oid != pending.snapshot.head_sha => {
                    "PR changed, reload"
                }
                Ok(status) if status.state == "OPEN" => "PR still open (possibly queued)",
                Ok(status) if status.state == "CLOSED" => "PR is closed",
                Ok(_) => "PR state is unknown",
                Err(_) => "could not verify PR state",
            };
            format!("{prefix}; {detail}. r refresh; do not resubmit.")
        };
        self.merge_message_for(&pending.snapshot, message, true);
        if let View::Review(review) = &mut self.view {
            if review
                .snapshot
                .as_ref()
                .is_some_and(|s| s.url == pending.snapshot.url)
            {
                review.merged = merged;
                review.readiness_started = Some(Instant::now());
                review.readiness = Some(
                    result
                        .map_err(|_| "Could not verify PR state".into())
                        .and_then(|status| {
                            if merged
                                || review
                                    .snapshot
                                    .as_ref()
                                    .is_some_and(|s| s.head_sha == status.head_ref_oid)
                            {
                                Ok(status)
                            } else {
                                Err("PR head changed; reopen PR to reload diff".into())
                            }
                        }),
                );
                if merged {
                    review.merge_message = None;
                }
            }
        }
    }

    fn open_states(&mut self) {
        if self.pending_mutation.is_some() {
            self.status = Some("A state change is already in progress.".into());
            return;
        }
        let Some((account, issue)) = self.selected() else {
            return;
        };
        let (issue_id, title, team, current_state) = (
            issue.id.clone(),
            issue.identifier.clone(),
            issue.team.id.clone(),
            issue.state.id.clone(),
        );
        let key = match &self.accounts[account].key {
            Ok(key) => key.clone(),
            Err(error) => {
                self.status = Some(error.clone());
                return;
            }
        };
        match Job::spawn(move || linear::fetch_states(&key, &team)) {
            Ok(job) => {
                self.view = View::StatePicker(StatePicker {
                    account,
                    issue_id,
                    title,
                    options: Vec::new(),
                    current_state,
                    started: Instant::now(),
                    confirming: false,
                    list: ListState::default(),
                    pending: Some(job),
                    error: None,
                })
            }
            Err(error) => self.status = Some(error),
        }
    }

    fn start_mutation(
        &mut self,
        account: usize,
        issue_id: String,
        title: String,
        state_id: String,
    ) {
        let key = match &self.accounts[account].key {
            Ok(key) => key.clone(),
            Err(error) => {
                self.status = Some(error.clone());
                return;
            }
        };
        let id = issue_id.clone();
        match Job::spawn(move || linear::change_state(&key, &id, &state_id)) {
            Ok(job) => {
                // A refresh started before this write must not overwrite the mutation result.
                self.accounts[account].list.pending = None;
                self.pending_mutation = Some(PendingMutation {
                    account,
                    issue_id,
                    title,
                    job,
                });
                self.status =
                    Some("Updating issue state… (cannot cancel a submitted change)".into());
            }
            Err(error) => self.status = Some(error),
        }
    }

    fn poll_mutation(&mut self) {
        let Some(result) = self.pending_mutation.as_ref().and_then(|p| p.job.poll()) else {
            return;
        };
        let pending = self
            .pending_mutation
            .take()
            .expect("pending mutation exists");
        let account = &mut self.accounts[pending.account];
        match result {
            Ok(state) => {
                self.status = Some(format!(
                    "{} / {} → {}",
                    account.name, pending.title, state.name
                ));
                if let Some(issue) = account
                    .list
                    .issues
                    .iter_mut()
                    .find(|i| i.id == pending.issue_id)
                {
                    issue.state = state;
                }
                let issues = account
                    .list
                    .issues
                    .iter()
                    .filter(|i| i.is_open())
                    .cloned()
                    .collect();
                account.list.apply_snapshot(issues);
            }
            Err(error) if error.contains(linear::UNKNOWN_OUTCOME) => {
                // A timeout may follow a successful write; never claim it failed.
                self.status = Some(format!(
                    "{} / {}: state change outcome unknown; refreshing to confirm",
                    account.name, pending.title
                ))
            }
            Err(error) => {
                self.status = Some(format!("{} / {}: {error}", account.name, pending.title))
            }
        }
        account.refresh(self.scope);
    }
}
