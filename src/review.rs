use std::{process::Command, sync::OnceLock};

use anyhow::{bail, Context, Result};
use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
    widgets::ListState,
};
use serde::Deserialize;
use syntect::{easy::HighlightLines, highlighting::ThemeSet, parsing::SyntaxSet};

use crate::{github::pr_args, merge::MergeMethod, worker::Job};

type FullFile = Vec<Line<'static>>;

#[derive(Clone)]
pub struct FileSource {
    repository: String,
    revision: String,
    path: String,
}

pub struct FileDiff {
    pub path: String,
    pub additions: usize,
    pub deletions: usize,
    pub lines: Vec<String>,
    pub source: Option<FileSource>,
    pub full: Option<Result<FullFile, String>>,
}

/// Identity of the immutable diff being read, not the latest PR head.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrSnapshot {
    pub original_url: String,
    pub url: String,
    pub repository: String,
    pub number: u64,
    pub title: String,
    pub head_sha: String,
    pub base_sha: String,
}

pub struct LoadedReview {
    pub snapshot: PrSnapshot,
    pub files: Vec<FileDiff>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Readiness {
    pub head_ref_oid: String,
    pub state: String,
    pub is_draft: bool,
    pub mergeable: String,
    pub review_decision: Option<String>,
    status_check_rollup: Option<Vec<Check>>,
}

#[derive(Debug, Deserialize)]
struct Check {
    #[serde(default)]
    status: String,
    #[serde(default)]
    conclusion: Option<String>,
    #[serde(default)]
    state: String,
}

impl Readiness {
    pub fn checks(&self) -> &'static str {
        let Some(checks) = self.status_check_rollup.as_ref().filter(|c| !c.is_empty()) else {
            return "none";
        };
        let mut pending = false;
        let mut unknown = false;
        for check in checks {
            let outcome = if !check.state.is_empty() {
                check.state.as_str()
            } else if check.status == "COMPLETED" {
                check.conclusion.as_deref().unwrap_or("")
            } else {
                check.status.as_str()
            };
            match outcome {
                "FAILURE" | "ERROR" | "CANCELLED" | "TIMED_OUT" | "ACTION_REQUIRED"
                | "STARTUP_FAILURE" | "STALE" => return "failed",
                "SUCCESS" | "NEUTRAL" | "SKIPPED" => {}
                "PENDING" | "QUEUED" | "IN_PROGRESS" | "WAITING" | "REQUESTED" => pending = true,
                _ => unknown = true,
            }
        }
        if unknown {
            "unknown"
        } else if pending {
            "pending"
        } else {
            "passed"
        }
    }

    pub fn summary(&self) -> String {
        let state = match self.state.as_str() {
            "MERGED" => "merged",
            "CLOSED" => "closed",
            "OPEN" if self.is_draft => "draft",
            "OPEN" => "open",
            _ => "unknown",
        };
        let mergeable = match self.mergeable.as_str() {
            "CONFLICTING" => "conflicting",
            "MERGEABLE" => "no conflicts",
            _ => "conflicts unknown",
        };
        let decision = match self.review_decision.as_deref() {
            Some("APPROVED") => "approved",
            Some("CHANGES_REQUESTED") => "changes requested",
            Some("REVIEW_REQUIRED") => "required",
            None | Some("") => "none",
            _ => "unknown",
        };
        format!(
            "{state} · {mergeable} · checks {} · review {decision}",
            self.checks()
        )
    }
}

pub fn load_readiness(pr_url: &str) -> Result<Readiness> {
    let args = pr_args(
        pr_url,
        "view",
        &[
            "--json",
            "headRefOid,state,isDraft,mergeable,reviewDecision,statusCheckRollup",
        ],
    )?;
    serde_json::from_slice(&gh(&args)?).context("could not read PR status")
}

pub struct MergePicker {
    pub options: Vec<MergeMethod>,
    pub list: ListState,
    pub confirming: bool,
    pub loading: bool,
    pub error: Option<String>,
}

pub struct Review {
    pub title: String,
    pub merge_picker: Option<MergePicker>,
    pub merged: bool,
    pub merge_message: Option<String>,
    pub snapshot: Option<Box<PrSnapshot>>,
    pub readiness: Option<Result<Readiness, String>>,
    pub readiness_loading: bool,
    pub readiness_started: Option<std::time::Instant>,
    pub files: Vec<FileDiff>,
    pub list: ListState,
    pub scroll: u16,
    pub horizontal: u16,
    pub full_file: bool,
    pending_file: Option<(usize, Job<FullFile>)>,
    file_started: std::time::Instant,
}

impl Review {
    pub fn new(title: String, files: Vec<FileDiff>) -> Self {
        let mut list = ListState::default();
        if !files.is_empty() {
            list.select(Some(0));
        }
        Self {
            title,
            merge_picker: None,
            merged: false,
            merge_message: None,
            snapshot: None,
            readiness: None,
            readiness_loading: false,
            readiness_started: None,
            files,
            list,
            scroll: 0,
            horizontal: 0,
            full_file: false,
            pending_file: None,
            file_started: std::time::Instant::now(),
        }
    }

    pub fn from_loaded(title: String, loaded: LoadedReview) -> Self {
        let mut review = Self::new(title, loaded.files);
        review.snapshot = Some(Box::new(loaded.snapshot));
        review
    }

    pub fn can_merge(&self) -> bool {
        !self.merged
            && !self.readiness_loading
            && self.snapshot.as_ref().is_some_and(|snapshot| {
                self.readiness.as_ref().is_some_and(|result| {
                    result.as_ref().is_ok_and(|status| {
                        status.head_ref_oid == snapshot.head_sha
                            && status.state == "OPEN"
                            && !status.is_draft
                            && status.mergeable != "CONFLICTING"
                    })
                })
            })
    }

    pub fn readiness_line(&self) -> String {
        if self.merged {
            return "Merged · merging disabled".into();
        }
        let Some(snapshot) = &self.snapshot else {
            return "PR status unavailable (no snapshot)".into();
        };
        let head: String = snapshot.head_sha.chars().take(12).collect();
        let status = match &self.readiness {
            Some(Ok(status)) => status.summary(),
            Some(Err(error)) => format!("status failed: {}", error.lines().next().unwrap_or(error)),
            None => "loading status…".into(),
        };
        crate::display::safe(&format!(
            "head {head} · {status}{}{}",
            if self.readiness_loading && self.readiness.is_some() {
                " · refreshing…"
            } else {
                ""
            },
            self.merge_message
                .as_ref()
                .map(|message| format!(" · {message}"))
                .unwrap_or_default()
        ))
    }

    pub fn current(&self) -> Option<&FileDiff> {
        self.list.selected().and_then(|i| self.files.get(i))
    }

    pub fn select_next(&mut self) {
        if !self.files.is_empty() {
            self.list.select(Some(
                (self.list.selected().unwrap_or(0) + 1).min(self.files.len() - 1),
            ));
        }
        self.reset_scroll();
        self.ensure_full_file();
    }

    pub fn select_previous(&mut self) {
        self.list
            .select(self.list.selected().map(|i| i.saturating_sub(1)));
        self.reset_scroll();
        self.ensure_full_file();
    }

    fn reset_scroll(&mut self) {
        self.scroll = 0;
        self.horizontal = 0;
    }

    pub fn toggle_full_file(&mut self) {
        self.full_file = !self.full_file;
        self.reset_scroll();
        self.ensure_full_file();
    }

    fn ensure_full_file(&mut self) {
        if !self.full_file {
            return;
        }
        let Some(index) = self.list.selected().filter(|&i| i < self.files.len()) else {
            return;
        };
        if self.files[index].full.as_ref().is_some_and(Result::is_ok) || self.pending_file.is_some()
        {
            return;
        }
        let Some(source) = self.files[index].source.clone() else {
            self.files[index].full = Some(Err("Full file is unavailable for this diff.".into()));
            return;
        };
        self.files[index].full = None;
        match Job::spawn(move || load_file(&source)) {
            Ok(job) => {
                self.file_started = std::time::Instant::now();
                self.pending_file = Some((index, job));
            }
            Err(error) => self.files[index].full = Some(Err(error)),
        }
    }

    #[cfg(test)]
    pub fn begin_test_file_load(&mut self) -> std::sync::mpsc::Sender<Result<FullFile, String>> {
        let (sender, job) = Job::channel();
        self.full_file = true;
        self.pending_file = Some((self.list.selected().unwrap_or(0), job));
        sender
    }

    /// When the selected file's full contents started loading, if they are loading.
    pub fn file_loading_since(&self) -> Option<std::time::Instant> {
        let (index, _) = self.pending_file.as_ref()?;
        (self.list.selected() == Some(*index)).then_some(self.file_started)
    }

    pub fn poll_file(&mut self) {
        let Some((index, job)) = &self.pending_file else {
            return;
        };
        if let Some(result) = job.poll() {
            let completed = *index;
            self.files[completed].full = Some(result);
            self.pending_file = None;
            if self.list.selected() != Some(completed) {
                self.ensure_full_file();
            }
        }
    }

    pub fn scroll_by(&mut self, delta: i32) {
        let len = self.current().map_or(0, |f| {
            if self.full_file {
                f.full
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .map_or(0, Vec::len)
            } else {
                f.lines.len()
            }
        });
        self.scroll = (self.scroll as i32 + delta)
            .clamp(0, len.saturating_sub(1).min(u16::MAX as usize) as i32)
            as u16;
    }

    pub fn scroll_horizontal(&mut self, delta: i32) {
        let width = self.current().map_or(0, |file| {
            if self.full_file {
                file.full
                    .as_ref()
                    .and_then(|r| r.as_ref().ok())
                    .map_or(0, |lines| lines.iter().map(Line::width).max().unwrap_or(0))
            } else {
                file.lines
                    .iter()
                    .map(|s| Line::from(s.as_str()).width())
                    .max()
                    .unwrap_or(0)
            }
        });
        self.horizontal = (self.horizontal as i32)
            .saturating_add(delta)
            .clamp(0, width.saturating_sub(1).min(u16::MAX as usize) as i32)
            as u16;
    }
}

fn gh(args: &[&str]) -> Result<Vec<u8>> {
    let output = crate::process::run(
        Command::new("gh").args(args).env("GH_PROMPT_DISABLED", "1"),
        std::time::Duration::from_secs(30),
    )
    .context("could not run gh (is GitHub CLI installed?)")?;
    if !output.status.success() {
        bail!("gh failed: {}", first_line(&output.stderr));
    }
    Ok(output.stdout)
}

fn first_line(stderr: &[u8]) -> String {
    String::from_utf8_lossy(stderr)
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("unknown error")
        .to_string()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Metadata {
    url: String,
    title: String,
    head_ref_oid: String,
    base_ref_oid: String,
    head_repository: Option<HeadRepository>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HeadRepository {
    name_with_owner: String,
}

pub fn load(pr_url: &str) -> Result<LoadedReview> {
    let metadata_args = pr_args(
        pr_url,
        "view",
        &["--json", "url,title,headRefOid,baseRefOid,headRepository"],
    )?;
    let metadata: Metadata = serde_json::from_slice(&gh(&metadata_args)?)?;
    let repository = repository(&metadata.url)?;
    let diff = gh(&pr_args(pr_url, "diff", &["--color", "never"])?)?;
    // Refuse an inconsistent snapshot if the PR changed while its diff loaded.
    let latest: Metadata = serde_json::from_slice(&gh(&metadata_args)?)?;
    if metadata.head_ref_oid != latest.head_ref_oid || metadata.base_ref_oid != latest.base_ref_oid
    {
        bail!("PR changed while loading; press Enter to retry");
    }
    let mut files = parse(&String::from_utf8_lossy(&diff));
    attach_sources(&mut files, &metadata, &repository);
    let number = metadata
        .url
        .rsplit('/')
        .next()
        .context("missing PR number")?
        .parse()?;
    Ok(LoadedReview {
        snapshot: PrSnapshot {
            original_url: pr_url.to_string(),
            url: metadata.url,
            repository,
            number,
            title: metadata.title,
            head_sha: metadata.head_ref_oid,
            base_sha: metadata.base_ref_oid,
        },
        files,
    })
}

fn attach_sources(files: &mut [FileDiff], metadata: &Metadata, repository: &str) {
    for file in files {
        let deleted = file
            .lines
            .iter()
            .take_while(|l| !l.starts_with("@@"))
            .any(|l| l == "+++ /dev/null");
        let path = if deleted {
            file.lines
                .iter()
                .take_while(|l| !l.starts_with("@@"))
                .find_map(|l| l.strip_prefix("--- "))
                .map(git_path)
                .and_then(|p| p.strip_prefix("a/").map(str::to_string))
                .unwrap_or_else(|| file.path.clone())
        } else {
            file.path.clone()
        };
        file.source = Some(FileSource {
            repository: if deleted {
                repository.to_string()
            } else {
                metadata
                    .head_repository
                    .as_ref()
                    .map_or_else(|| repository.to_string(), |r| r.name_with_owner.clone())
            },
            revision: if deleted {
                metadata.base_ref_oid.clone()
            } else {
                metadata.head_ref_oid.clone()
            },
            path,
        });
    }
}

fn repository(url: &str) -> Result<String> {
    let parts: Vec<_> = url
        .strip_prefix("https://github.com/")
        .context("not a GitHub PR URL")?
        .split('/')
        .collect();
    crate::github::repository(url)?;
    if parts.len() != 4
        || parts[2] != "pull"
        || parts[..2].iter().any(|s| {
            s.is_empty()
                || !s
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
        })
    {
        bail!("invalid GitHub PR URL");
    }
    Ok(format!("{}/{}", parts[0], parts[1]))
}

fn encode_path(path: &str) -> String {
    path.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || b"-._~/".contains(&b) {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn load_file(source: &FileSource) -> Result<FullFile> {
    // Git blobs are immutable and work for fork heads as well as deleted base files.
    #[derive(Deserialize)]
    struct Entry {
        sha: String,
        size: Option<usize>,
        #[serde(rename = "type")]
        kind: String,
    }
    let endpoint = format!(
        "repos/{}/contents/{}?ref={}",
        source.repository,
        encode_path(&source.path),
        source.revision
    );
    let entry: Entry = serde_json::from_slice(&gh(&["api", &endpoint])?)?;
    if entry.kind != "file" {
        bail!("Not a regular file (directory or submodule).");
    }
    if entry.size.unwrap_or(0) > 2_000_000 {
        bail!("File exceeds the 2 MB preview limit.");
    }
    let blob = format!("repos/{}/git/blobs/{}", source.repository, entry.sha);
    let bytes = gh(&[
        "api",
        &blob,
        "-H",
        "Accept: application/vnd.github.raw+json",
    ])?;
    preview(&source.path, &bytes)
}

fn preview(path: &str, bytes: &[u8]) -> Result<FullFile> {
    if bytes.len() > 2_000_000 {
        bail!("File exceeds the 2 MB preview limit.");
    }
    let text = std::str::from_utf8(bytes).context("Binary/non-UTF-8 file cannot be displayed")?;
    if text.contains('\0') {
        bail!("Binary file cannot be displayed");
    }
    Ok(highlight(path, text))
}

fn highlight(path: &str, text: &str) -> FullFile {
    static SYNTAXES: OnceLock<SyntaxSet> = OnceLock::new();
    static THEMES: OnceLock<ThemeSet> = OnceLock::new();
    let syntaxes = SYNTAXES.get_or_init(SyntaxSet::load_defaults_newlines);
    let themes = THEMES.get_or_init(ThemeSet::load_defaults);
    let syntax = std::path::Path::new(path)
        .extension()
        .and_then(|ext| ext.to_str())
        .and_then(|ext| syntaxes.find_syntax_by_extension(ext))
        .or_else(|| {
            text.lines()
                .next()
                .and_then(|line| syntaxes.find_syntax_by_first_line(line))
        })
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text());
    let mut highlighter = HighlightLines::new(syntax, &themes.themes["base16-ocean.dark"]);
    text.lines()
        .map(|line| {
            // Tabs become fixed-width spaces in both full-file and diff modes.
            let line = crate::display::safe(&line.replace('\t', "    "));
            match highlighter.highlight_line(&(line.clone() + "\n"), syntaxes) {
                Ok(ranges) => Line::from(
                    ranges
                        .into_iter()
                        .map(|(style, part)| {
                            Span::styled(
                                part.trim_end_matches('\n').to_string(),
                                Style::new().fg(Color::Rgb(
                                    style.foreground.r,
                                    style.foreground.g,
                                    style.foreground.b,
                                )),
                            )
                        })
                        .collect::<Vec<_>>(),
                ),
                Err(_) => Line::from(line),
            }
        })
        .collect()
}

// Git quotes non-ASCII filenames using C-style/octal byte escapes.
fn quoted_path(input: &str) -> (String, &str) {
    let bytes = input.as_bytes();
    let mut out = Vec::new();
    let mut i = 1;
    while i < bytes.len() {
        let b = bytes[i];
        i += 1;
        if b == b'"' {
            break;
        }
        if b != b'\\' {
            out.push(b);
            continue;
        }
        if i >= bytes.len() {
            break;
        }
        let escaped = bytes[i];
        i += 1;
        if (b'0'..=b'7').contains(&escaped) {
            let mut value = (escaped - b'0') as u16;
            for _ in 0..2 {
                if i < bytes.len() && (b'0'..=b'7').contains(&bytes[i]) {
                    value = value * 8 + (bytes[i] - b'0') as u16;
                    i += 1;
                } else {
                    break;
                }
            }
            out.push(value as u8);
        } else {
            out.push(match escaped {
                b't' => b'\t',
                b'n' => b'\n',
                b'r' => b'\r',
                other => other,
            });
        }
    }
    (String::from_utf8_lossy(&out).into_owned(), &input[i..])
}
fn git_path(input: &str) -> String {
    if input.starts_with('"') {
        quoted_path(input).0
    } else {
        input.to_string()
    }
}

pub fn parse(diff: &str) -> Vec<FileDiff> {
    let mut files: Vec<FileDiff> = Vec::new();
    let mut in_hunk = false;
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("diff --git ") {
            in_hunk = false;
            let path = if rest.starts_with('"') {
                let (_, remaining) = quoted_path(rest);
                git_path(remaining.trim_start())
                    .strip_prefix("b/")
                    .unwrap_or(remaining)
                    .to_string()
            } else if let Some((_, path)) = rest.rsplit_once(" \"b/") {
                git_path(&format!("\"b/{path}"))
                    .trim_start_matches("b/")
                    .to_string()
            } else {
                rest.rsplit_once(" b/").map_or(rest, |(_, p)| p).to_string()
            };
            files.push(FileDiff {
                path,
                additions: 0,
                deletions: 0,
                lines: Vec::new(),
                source: None,
                full: None,
            });
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if line.starts_with("@@") {
            in_hunk = true;
        }
        if let Some(path) = line
            .strip_prefix("+++ ")
            .filter(|p| !in_hunk && *p != "/dev/null")
        {
            let path = git_path(path);
            file.path = path.strip_prefix("b/").unwrap_or(&path).to_string();
        }
        if line.starts_with('+') && (in_hunk || !line.starts_with("+++")) {
            file.additions += 1;
        } else if line.starts_with('-') && (in_hunk || !line.starts_with("---")) {
            file.deletions += 1;
        }
        file.lines.push(line.replace('\t', "    "));
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[ignore = "requires gh auth and HERDR_LINEAR_TEST_PR"]
    fn github_live_full_file_snapshot() {
        let url = std::env::var("HERDR_LINEAR_TEST_PR").expect("set a test PR URL");
        let loaded = load(&url).unwrap();
        let source = loaded
            .files
            .iter()
            .find(|f| f.path.ends_with(".rs"))
            .unwrap()
            .source
            .as_ref()
            .unwrap();
        let lines = load_file(source).unwrap();
        assert!(!lines.is_empty());
        assert!(lines.iter().any(|l| l.spans.len() > 1));
    }
    #[test]
    fn deleted_files_pin_base_and_renames_pin_fork_head() {
        let mut files=parse("diff --git a/old.rs b/old.rs\n--- a/old.rs\n+++ /dev/null\n-old\ndiff --git a/name.rs b/new.rs\n--- a/name.rs\n+++ b/new.rs\n+new\n");
        let metadata = Metadata {
            url: "https://github.com/o/r/pull/1".into(),
            title: "Test PR".into(),
            head_ref_oid: "head".into(),
            base_ref_oid: "base".into(),
            head_repository: Some(HeadRepository {
                name_with_owner: "fork/r".into(),
            }),
        };
        attach_sources(&mut files, &metadata, "o/r");
        let deleted = files[0].source.as_ref().unwrap();
        assert_eq!(
            (&*deleted.repository, &*deleted.revision, &*deleted.path),
            ("o/r", "base", "old.rs")
        );
        let renamed = files[1].source.as_ref().unwrap();
        assert_eq!(
            (&*renamed.repository, &*renamed.revision, &*renamed.path),
            ("fork/r", "head", "new.rs")
        );
    }
    #[test]
    fn binary_non_utf8_and_large_files_are_explained() {
        assert!(preview("a", b"binary\0data").is_err());
        assert!(preview("a", &[255]).is_err());
        assert!(preview("a", &vec![b'x'; 2_000_001]).is_err());
        assert!(preview("a", b"").unwrap().is_empty());
    }
    #[test]
    fn quoted_unicode_and_renamed_paths_are_decoded() {
        let files=parse("diff --git \"a/\\355\\225\\234.rs\" \"b/\\355\\225\\234.rs\"\n--- \"a/\\355\\225\\234.rs\"\n+++ \"b/\\355\\225\\234.rs\"\n+hello\n");
        assert_eq!(files[0].path, "한.rs");
        let renamed = parse(
            "diff --git a/old name.rs b/new name.rs\n--- a/old name.rs\n+++ b/new name.rs\n+new\n",
        );
        assert_eq!(renamed[0].path, "new name.rs");
    }
    #[test]
    fn header_like_hunk_content_is_counted_and_never_overwrites_paths() {
        let diff = "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@\n+++i;\n--- note\n+++ b/foo\n--- a/bar\n";
        let files = parse(diff);
        assert_eq!(files[0].path, "src/a.rs");
        assert_eq!((files[0].additions, files[0].deletions), (2, 2));
        let sql = parse("diff --git a/q.sql b/q.sql\n--- a/q.sql\n+++ b/q.sql\n@@ -1 +0,0 @@\n--- sql comment\n");
        assert_eq!((sql[0].additions, sql[0].deletions), (0, 1));
        let mut files = parse(diff);
        let metadata = Metadata {
            url: "https://github.com/o/r/pull/1".into(),
            title: "Test PR".into(),
            head_ref_oid: "head".into(),
            base_ref_oid: "base".into(),
            head_repository: None,
        };
        attach_sources(&mut files, &metadata, "o/r");
        assert_eq!(files[0].source.as_ref().unwrap().path, "src/a.rs");
    }
    fn source() -> FileSource {
        FileSource {
            repository: "o/r".into(),
            revision: "head".into(),
            path: "a.rs".into(),
        }
    }
    #[test]
    fn only_one_full_file_load_runs_and_stale_results_follow_selection() {
        let mut review = Review::new(
            "t".into(),
            parse("diff --git a/a b/a\n+1\ndiff --git a/b b/b\n+2\ndiff --git a/c b/c\n+3\n"),
        );
        for file in &mut review.files {
            file.source = Some(source());
        }
        review.full_file = true;
        let (first, job) = Job::channel();
        review.pending_file = Some((0, job));
        review.select_next();
        review.select_next();
        // Navigation while busy does not start more work.
        assert_eq!(review.pending_file.as_ref().map(|(i, _)| *i), Some(0));
        first.send(Ok(vec![Line::from("a")])).unwrap();
        review.poll_file();
        // The finished result is stored under its own file, never the selected one.
        assert!(review.files[0].full.as_ref().unwrap().is_ok());
        assert!(review.files[2].full.is_none());
        // The selected file is loaded next instead of being skipped.
        assert_eq!(review.pending_file.as_ref().map(|(i, _)| *i), Some(2));
    }
    #[test]
    fn failed_full_file_loads_are_retried() {
        let mut review = Review::new("t".into(), parse("diff --git a/a b/a\n+1\n"));
        review.files[0].source = Some(source());
        review.full_file = true;
        review.files[0].full = Some(Err("timed out".into()));
        review.toggle_full_file();
        review.toggle_full_file();
        assert!(
            review.pending_file.is_some(),
            "toggling back retries a failed load"
        );
        assert!(review.files[0].full.is_none());
    }
    #[test]
    fn horizontal_scroll_is_bounded_at_both_ends() {
        let mut review = Review::new("t".into(), parse("diff --git a/a b/a\n+0123456789\n"));
        review.scroll_horizontal(-8);
        assert_eq!(review.horizontal, 0);
        review.scroll_horizontal(i32::MAX);
        let widest = review.files[0]
            .lines
            .iter()
            .map(|l| l.chars().count())
            .max()
            .unwrap();
        assert_eq!(review.horizontal as usize, widest - 1);
    }
    #[test]
    fn gh_errors_show_their_first_line() {
        assert_eq!(
            first_line(b"\nGraphQL: Could not resolve\n  at line 2\n"),
            "GraphQL: Could not resolve"
        );
        assert_eq!(first_line(b""), "unknown error");
    }
    #[test]
    fn splits_files_and_counts_changes() {
        let files = parse("diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1 +1 @@\n-old\n+new\ndiff --git a/b b/b\n+added\n");
        assert_eq!(files.len(), 2);
        assert_eq!((files[0].additions, files[0].deletions), (1, 1));
    }
    #[test]
    fn syntax_highlighting_preserves_unicode_and_plain_text() {
        let lines = highlight("a.rs", "fn main() { println!(\"안녕\"); }\n");
        assert_eq!(lines[0].to_string(), "fn main() { println!(\"안녕\"); }");
        assert!(lines[0].spans.len() > 1);
        assert_eq!(highlight("file.unknown", "hello")[0].to_string(), "hello");
    }
    #[test]
    fn scroll_clamps_and_resets_on_file_change() {
        let mut review = Review::new("test".into(), parse("diff --git a/a b/a\n+line\n"));
        review.scroll_by(i32::MAX - 65535);
        assert_eq!(review.scroll, 1);
        review.scroll_horizontal(10);
        review.select_next();
        assert_eq!(review.horizontal, 0);
        review.toggle_full_file();
        assert!(review.current().unwrap().full.as_ref().unwrap().is_err());
    }
    #[test]
    fn escapes_content_paths_and_validates_repositories() {
        assert_eq!(encode_path("src/a #한.rs"), "src/a%20%23%ED%95%9C.rs");
        assert_eq!(repository("https://github.com/o/r/pull/1").unwrap(), "o/r");
        assert!(repository("https://example.com/o/r/pull/1").is_err());
    }
}
