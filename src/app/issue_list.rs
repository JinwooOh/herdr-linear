use std::time::Instant;

use ratatui::widgets::ListState;

use crate::{linear, worker::Job};

/// A Linear fetch for one list of issues, given the Account's API key.
pub type Fetch = fn(&str) -> anyhow::Result<Vec<linear::Issue>>;

/// One list of an Account's issues and everything tracked about loading it.
pub struct IssueList {
    pub issues: Vec<linear::Issue>,
    pub selection: ListState,
    pub error: Option<String>,
    pub(super) pending: Option<Job<Vec<linear::Issue>>>,
    pub(super) refreshed: Instant,
}

impl Default for IssueList {
    fn default() -> Self {
        Self {
            issues: Vec::new(),
            selection: ListState::default(),
            error: None,
            pending: None,
            refreshed: Instant::now(),
        }
    }
}

impl IssueList {
    /// Starts `fetch` in the background unless a fetch for this list is already running.
    pub(super) fn refresh(&mut self, key: &Result<String, String>, fetch: Fetch) {
        if self.pending.is_some() {
            return;
        }
        self.refreshed = Instant::now();
        let key = match key {
            Ok(key) => key.clone(),
            Err(error) => {
                self.error = Some(error.clone());
                return;
            }
        };
        match Job::spawn(move || fetch(&key)) {
            Ok(job) => self.pending = Some(job),
            Err(error) => self.error = Some(error),
        }
    }

    /// Applies the running fetch's result once it has arrived.
    pub(super) fn poll(&mut self) {
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

    pub(super) fn apply_snapshot(&mut self, mut issues: Vec<linear::Issue>) {
        let selected_id = self.selected().map(|issue| issue.id.clone());
        let old_index = self.selection.selected().unwrap_or(0);
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
        self.selection
            .select(if issues.is_empty() { None } else { Some(index) });
        self.issues = issues;
    }

    pub fn selected(&self) -> Option<&linear::Issue> {
        self.selection.selected().and_then(|i| self.issues.get(i))
    }

    pub fn loading(&self) -> bool {
        self.pending.is_some()
    }
}

#[cfg(test)]
mod tests;
