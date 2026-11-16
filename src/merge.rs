use std::{process::Command, time::Duration};

use anyhow::{bail, Context, Result};
use serde::Deserialize;

use crate::{github, review::PrSnapshot};

#[derive(Clone, Copy, Debug, Default, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum MergeMethod {
    Merge,
    #[default]
    Squash,
    Rebase,
}

impl MergeMethod {
    pub fn name(self) -> &'static str {
        match self {
            Self::Merge => "merge",
            Self::Squash => "squash",
            Self::Rebase => "rebase",
        }
    }

    fn flag(self) -> &'static str {
        match self {
            Self::Merge => "--merge",
            Self::Squash => "--squash",
            Self::Rebase => "--rebase",
        }
    }
}

#[derive(Deserialize)]
struct RepositoryMethods {
    allow_merge_commit: bool,
    allow_squash_merge: bool,
    allow_rebase_merge: bool,
}

pub fn load_methods(snapshot: &PrSnapshot) -> Result<Vec<MergeMethod>> {
    let repository = github::repository(&snapshot.url)?;
    let endpoint = format!("repos/{repository}");
    let output = gh(&["api", &endpoint])?;
    let allowed: RepositoryMethods =
        serde_json::from_slice(&output).context("could not read repository merge methods")?;
    Ok(allowed.methods())
}

impl RepositoryMethods {
    fn methods(self) -> Vec<MergeMethod> {
        [
            (MergeMethod::Merge, self.allow_merge_commit),
            (MergeMethod::Squash, self.allow_squash_merge),
            (MergeMethod::Rebase, self.allow_rebase_merge),
        ]
        .into_iter()
        .filter_map(|(method, allowed)| allowed.then_some(method))
        .collect()
    }
}

/// Explicit method and immutable head only: never bypass protections, request
/// auto-merge, or delete a branch. GitHub may still enqueue the PR; verify state.
pub fn command_args(snapshot: &PrSnapshot, method: MergeMethod) -> Result<Vec<&str>> {
    if snapshot.head_sha.is_empty() || !snapshot.head_sha.bytes().all(|b| b.is_ascii_hexdigit()) {
        bail!("invalid PR head commit");
    }
    github::pr_args(
        &snapshot.url,
        "merge",
        &[method.flag(), "--match-head-commit", &snapshot.head_sha],
    )
}

/// A successful command is only an acknowledgement, not proof of a merge.
#[derive(Debug, PartialEq, Eq)]
pub enum Submission {
    Accepted,
    Changed,
    Refused(String),
    Unknown,
}

pub fn submit(snapshot: &PrSnapshot, method: MergeMethod) -> Result<Submission> {
    submit_with(snapshot, method, crate::review::load_readiness, |args| {
        crate::process::run(
            Command::new("gh").args(args).env("GH_PROMPT_DISABLED", "1"),
            Duration::from_secs(30),
        )
    })
}

fn submit_with(
    snapshot: &PrSnapshot,
    method: MergeMethod,
    readiness: impl FnOnce(&str) -> Result<crate::review::Readiness>,
    run: impl FnOnce(&[&str]) -> Result<std::process::Output>,
) -> Result<Submission> {
    let args = command_args(snapshot, method)?;
    let status = match readiness(&snapshot.url) {
        Ok(status) => status,
        Err(_) => {
            return Ok(Submission::Refused(
                "Could not check PR status; no merge submitted.".into(),
            ))
        }
    };
    if status.head_ref_oid != snapshot.head_sha {
        return Ok(Submission::Changed);
    }
    if let Some(reason) = status.merge_unavailable(snapshot) {
        return Ok(Submission::Refused(reason.into()));
    }
    // The preflight is advisory. --match-head-commit closes the race with pushes
    // after that read; never substitute a newly fetched head for the reviewed one.
    Ok(match run(&args) {
        Ok(output) if output.status.success() => Submission::Accepted,
        Ok(output) if output.status.code().is_some() => classify_refusal(&output.stderr),
        Ok(_) => Submission::Unknown,
        Err(_) => Submission::Unknown,
    })
}

fn classify_refusal(stderr: &[u8]) -> Submission {
    let text = String::from_utf8_lossy(stderr);
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let lower = line.to_ascii_lowercase();
    if lower.contains("head")
        && (lower.contains("does not match --match-head-commit")
            || lower.contains("head branch was modified"))
    {
        return Submission::Changed;
    }
    // Only explicit server/CLI refusals are known failures. Transport errors,
    // signals, output limits and unrecognized diagnostics may follow a write.
    if [
        "branch protection",
        "protected branch",
        "base branch policy",
        "required status check",
        "pending checks",
        "checks have not passed",
        "not mergeable",
        "must be approved",
        "review is required",
        "review required",
        "still a draft",
        "is closed",
        "already merged",
    ]
    .iter()
    .any(|reason| lower.contains(reason))
        || (lower.starts_with("graphql:")
            && [
                "permission",
                "not authorized",
                "not permitted",
                "resource not accessible",
            ]
            .iter()
            .any(|reason| lower.contains(reason)))
        || lower.starts_with("http 401:")
        || lower.starts_with("http 403:")
    {
        // Deliberate exception to generic subprocess diagnostics: show only the
        // refusal's first line, bounded and made safe for terminal rendering.
        Submission::Refused(crate::display::safe(
            &line.chars().take(500).collect::<String>(),
        ))
    } else {
        Submission::Unknown
    }
}

fn gh(args: &[&str]) -> Result<Vec<u8>> {
    let output = crate::process::run(
        Command::new("gh").args(args).env("GH_PROMPT_DISABLED", "1"),
        Duration::from_secs(30),
    )
    .context("could not run gh")?;
    if !output.status.success() {
        // Do not expose subprocess output, which may contain private data.
        bail!("GitHub request failed; refresh PR status before retrying");
    }
    Ok(output.stdout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repository_flags_expose_only_allowed_methods_in_stable_order() {
        use MergeMethod::*;
        for flags in 0..8 {
            let allowed: RepositoryMethods = serde_json::from_value(serde_json::json!({
                "allow_merge_commit": flags & 1 != 0,
                "allow_squash_merge": flags & 2 != 0,
                "allow_rebase_merge": flags & 4 != 0,
            }))
            .unwrap();
            let methods = allowed.methods();
            let expected: Vec<_> = [Merge, Squash, Rebase]
                .into_iter()
                .enumerate()
                .filter_map(|(i, method)| (flags & (1 << i) != 0).then_some(method))
                .collect();
            assert_eq!(methods, expected);
        }
        assert!(serde_json::from_str::<RepositoryMethods>("{}").is_err());
    }

    fn snapshot() -> PrSnapshot {
        PrSnapshot {
            original_url: "https://github.com/o/r/pull/1".into(),
            url: "https://github.com/o/r/pull/1".into(),
            repository: "o/r".into(),
            number: 1,
            title: "Example".into(),
            head_sha: "abcdef0123456789".into(),
            base_sha: "0123456789abcdef".into(),
        }
    }

    fn readiness(head: &str) -> crate::review::Readiness {
        serde_json::from_value(serde_json::json!({
            "headRefOid": head, "state": "OPEN", "isDraft": false,
            "mergeable": "MERGEABLE", "reviewDecision": null,
            "statusCheckRollup": []
        }))
        .unwrap()
    }

    #[test]
    fn preflight_head_mismatch_or_unavailable_status_never_invokes_a_write() {
        let snapshot = snapshot();
        let outcome = submit_with(
            &snapshot,
            MergeMethod::Squash,
            |_| Ok(readiness("different")),
            |_| panic!("must not merge a changed head"),
        )
        .unwrap();
        assert_eq!(outcome, Submission::Changed);
        for (state, draft, mergeable) in [
            ("MERGED", false, "MERGEABLE"),
            ("CLOSED", false, "MERGEABLE"),
            ("OPEN", true, "MERGEABLE"),
            ("OPEN", false, "CONFLICTING"),
        ] {
            let outcome = submit_with(
                &snapshot,
                MergeMethod::Squash,
                |_| {
                    let mut status = readiness(&snapshot.head_sha);
                    status.state = state.into();
                    status.is_draft = draft;
                    status.mergeable = mergeable.into();
                    Ok(status)
                },
                |_| panic!("must not write an unavailable PR"),
            )
            .unwrap();
            assert!(matches!(outcome, Submission::Refused(_)));
        }
        let outcome = submit_with(
            &snapshot,
            MergeMethod::Squash,
            |_| bail!("read failed"),
            |_| panic!("must not write without preflight"),
        )
        .unwrap();
        assert!(matches!(outcome, Submission::Refused(_)));
    }

    #[test]
    fn preflight_never_changes_the_pinned_head_and_process_errors_are_uncertain() {
        let snapshot = snapshot();
        let outcome = submit_with(
            &snapshot,
            MergeMethod::Squash,
            |_| Ok(readiness(&snapshot.head_sha)),
            |args| {
                assert!(args
                    .windows(2)
                    .any(|pair| pair == ["--match-head-commit", &snapshot.head_sha]));
                bail!("output capture failed after write")
            },
        )
        .unwrap();
        assert_eq!(outcome, Submission::Unknown);
    }

    #[cfg(unix)]
    #[test]
    fn command_acknowledgement_refusal_head_race_and_signal_are_distinct() {
        use std::os::unix::process::ExitStatusExt;
        let snapshot = snapshot();
        for (status, stderr, expected) in [
            (0, "", Submission::Accepted),
            (
                256,
                "GraphQL: permission denied\nprivate",
                Submission::Refused("GraphQL: permission denied".into()),
            ),
            (
                256,
                "the head does not match --match-head-commit",
                Submission::Changed,
            ),
            (256, "connection reset after request", Submission::Unknown),
            (9, "GraphQL: permission denied", Submission::Unknown),
        ] {
            let outcome = submit_with(
                &snapshot,
                MergeMethod::Squash,
                |_| Ok(readiness(&snapshot.head_sha)),
                |_| {
                    Ok(std::process::Output {
                        status: std::process::ExitStatus::from_raw(status),
                        stdout: vec![],
                        stderr: stderr.as_bytes().to_vec(),
                    })
                },
            )
            .unwrap();
            assert_eq!(outcome, expected);
        }
    }

    #[test]
    fn only_known_refusals_expose_a_bounded_safe_first_line() {
        for line in [
            "GraphQL: branch protection requires approval",
            "required status check is pending",
            "GraphQL: Resource not accessible by integration (mergePullRequest)",
            "GraphQL: permission denied (mergePullRequest)",
        ] {
            assert_eq!(
                classify_refusal(format!("\n{line}\nprivate detail").as_bytes()),
                Submission::Refused(line.into())
            );
        }
        assert_eq!(
            classify_refusal(b"the head of pull request o/r#1 does not match --match-head-commit"),
            Submission::Changed
        );
        for error in [
            "connection reset",
            "HTTP 502: bad gateway",
            "unexpected EOF",
            "",
            "Killed",
            "could not save local state: permission denied",
        ] {
            assert_eq!(classify_refusal(error.as_bytes()), Submission::Unknown);
        }
        assert_eq!(
            classify_refusal(b"GraphQL: permission denied\x1b[31m\nprivate"),
            Submission::Refused("GraphQL: permission denied<U+001B>[31m".into())
        );
    }

    #[test]
    fn merge_command_is_explicit_pinned_and_never_privileged_or_automatic() {
        let snapshot = snapshot();
        for method in [MergeMethod::Merge, MergeMethod::Squash, MergeMethod::Rebase] {
            let args = command_args(&snapshot, method).unwrap();
            assert_eq!(
                args,
                [
                    "pr",
                    "merge",
                    method.flag(),
                    "--match-head-commit",
                    &snapshot.head_sha,
                    "--",
                    &snapshot.url
                ]
            );
            for forbidden in ["--admin", "--auto", "--delete-branch"] {
                assert!(!args.contains(&forbidden));
            }
        }
        let mut invalid = snapshot;
        invalid.url = "--help".into();
        assert!(command_args(&invalid, MergeMethod::Squash).is_err());
    }
}
