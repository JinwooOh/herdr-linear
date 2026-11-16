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

/// Explicit method and immutable head only: never bypass protections, enqueue an
/// automatic merge, or delete a branch. The URL is validated before invoking gh.
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

pub fn submit(snapshot: &PrSnapshot, method: MergeMethod) -> Result<()> {
    gh(&command_args(snapshot, method)?)?;
    Ok(())
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

    #[test]
    fn merge_command_is_explicit_pinned_and_never_privileged_or_automatic() {
        let snapshot = PrSnapshot {
            original_url: "https://github.com/o/r/pull/1".into(),
            url: "https://github.com/o/r/pull/1".into(),
            repository: "o/r".into(),
            number: 1,
            title: "Example".into(),
            head_sha: "abcdef0123456789".into(),
            base_sha: "0123456789abcdef".into(),
        };
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
