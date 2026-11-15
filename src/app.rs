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
    config::AccountConfig,
    linear,
    review::{self, Review},
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

pub struct Account {
    pub name: String,
    key: Result<String, String>,
    pub issues: Vec<linear::Issue>,
    pub list: ListState,
    pub error: Option<String>,
    pending: Option<Job<Vec<linear::Issue>>>,
    refreshed: Instant,
}

impl Account {
    fn new(config: AccountConfig) -> Self {
        let mut account = Self {
            name: config.name,
            key: config.key,
            issues: Vec::new(),
            list: ListState::default(),
            error: None,
            pending: None,
            refreshed: Instant::now(),
        };
        account.refresh();
        account
    }

    fn refresh(&mut self) {
        if self.pending.is_some() {
            return;
        }
        self.refreshed = Instant::now();
        let key = match &self.key {
            Ok(key) => key.clone(),
            Err(error) => {
                self.error = Some(error.clone());
                return;
            }
        };
        match Job::spawn(move || linear::fetch_issues(&key)) {
            Ok(job) => self.pending = Some(job),
            Err(error) => self.error = Some(error),
        }
    }

    fn poll(&mut self) {
        let Some(job) = &self.pending else { return };
        if let Some(result) = job.poll() {
            self.pending = None;
            self.refreshed = Instant::now();
            match result {
                Ok(issues) => {
                    self.apply_snapshot(issues);
                    self.error = None;
                }
                Err(error) => self.error = Some(error),
            }
        }
    }

    fn apply_snapshot(&mut self, mut issues: Vec<linear::Issue>) {
        let selected_id = self.selected().map(|issue| issue.id.clone());
        let old_index = self.list.selected().unwrap_or(0);
        // A complete membership snapshot reconciles completed/unassigned/deleted issues.
        // Keep unchanged objects by updatedAt; delta-only queries miss removals.
        let old: std::collections::HashMap<_, _> = std::mem::take(&mut self.issues)
            .into_iter()
            .map(|i| (i.id.clone(), i))
            .collect();
        for issue in &mut issues {
            if let Some(previous) = old
                .get(&issue.id)
                .filter(|old| old.updated_at == issue.updated_at)
            {
                *issue = previous.clone();
            }
        }
        issues.sort_by(|a, b| {
            b.is_in_review()
                .cmp(&a.is_in_review())
                .then(b.updated_at.cmp(&a.updated_at))
                .then(a.id.cmp(&b.id))
        });
        let index = selected_id
            .and_then(|id| issues.iter().position(|i| i.id == id))
            .unwrap_or(old_index.min(issues.len().saturating_sub(1)));
        self.list
            .select(if issues.is_empty() { None } else { Some(index) });
        self.issues = issues;
    }

    pub fn selected(&self) -> Option<&linear::Issue> {
        self.list.selected().and_then(|i| self.issues.get(i))
    }

    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }
}

#[cfg(test)]
mod tests;
#[cfg(test)]
mod workflow_tests;

struct PendingReview {
    title: String,
    started: Instant,
    result: Job<Vec<review::FileDiff>>,
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
    pub status: Option<String>,
    pub view: View,
    pub queue: Vec<(usize, usize)>,
    pub queue_list: ListState,
    quit: bool,
    pending_review: Option<PendingReview>,
    pending_mutation: Option<PendingMutation>,
}

impl App {
    pub fn new(configs: Vec<AccountConfig>) -> Self {
        Self {
            accounts: configs.into_iter().map(Account::new).collect(),
            active: 0,
            status: None,
            view: View::Issues,
            queue: Vec::new(),
            queue_list: ListState::default(),
            quit: false,
            pending_review: None,
            pending_mutation: None,
        }
    }

    pub fn all_accounts_queue(&self) -> bool {
        self.active == self.accounts.len()
    }

    pub fn selected(&self) -> Option<(usize, &linear::Issue)> {
        if self.all_accounts_queue() {
            let &(account, index) = self.queue_list.selected().and_then(|i| self.queue.get(i))?;
            Some((account, self.accounts.get(account)?.issues.get(index)?))
        } else {
            self.accounts
                .get(self.active)?
                .selected()
                .map(|issue| (self.active, issue))
        }
    }

    fn list_mut(&mut self) -> &mut ListState {
        if self.all_accounts_queue() {
            &mut self.queue_list
        } else {
            &mut self.accounts[self.active].list
        }
    }

    fn rebuild_queue(&mut self, selected: Option<(usize, String)>) {
        self.queue = self
            .accounts
            .iter()
            .enumerate()
            .flat_map(|(a, account)| (0..account.issues.len()).map(move |i| (a, i)))
            .collect();
        self.queue.sort_by(|&(a, i), &(b, j)| {
            let x = &self.accounts[a].issues[i];
            let y = &self.accounts[b].issues[j];
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
                    .position(|&(b, i)| a == b && self.accounts[b].issues[i].id == id)
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
            .map(|&(a, i)| (a, self.accounts[a].issues[i].id.clone()));
        for (index, account) in self.accounts.iter_mut().enumerate() {
            account.poll();
            if !self
                .pending_mutation
                .as_ref()
                .is_some_and(|m| m.account == index)
                && account.refreshed.elapsed() >= REFRESH_INTERVAL
            {
                account.refresh();
            }
        }
        self.poll_mutation();
        self.rebuild_queue(selected);
        self.poll_review();
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
        // Raw mode delivers Ctrl+C as a key; quit from any view.
        if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
            self.quit = true;
        } else {
            self.on_key(key.code);
        }
    }

    fn on_key(&mut self, code: KeyCode) {
        self.status = None;
        // The loading box is modal: only cancel and quit act, so a stray key
        // cannot silently abandon a slow load.
        if self.pending_review.is_some() {
            match code {
                KeyCode::Esc => self.cancel_review(),
                KeyCode::Char('q') => self.quit = true,
                _ => {}
            }
            return;
        }
        if matches!(self.view, View::PrPicker(_) | View::StatePicker(_)) {
            self.picker_key(code);
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
                _ => {}
            }
            return;
        }
        match code {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
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
                        account.refresh();
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
            _ => {}
        }
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
        load: impl FnOnce() -> Result<Vec<review::FileDiff>, String> + Send + 'static,
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
            Ok(files) => {
                self.status = None;
                self.view = View::Review(Review::new(pending.title, files));
            }
            Err(error) => self.status = Some(error),
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
                self.accounts[account].pending = None;
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
                if let Some(issue) = account.issues.iter_mut().find(|i| i.id == pending.issue_id) {
                    issue.state = state;
                }
                let issues = account
                    .issues
                    .iter()
                    .filter(|i| i.is_open())
                    .cloned()
                    .collect();
                account.apply_snapshot(issues);
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
        account.refresh();
    }
}
