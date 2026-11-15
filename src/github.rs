use anyhow::{bail, Result};

/// Deliberately accept only canonical GitHub PR URLs, not arbitrary gh selectors.
pub fn repository(url: &str) -> Result<String> {
    let parts: Vec<_> = url
        .strip_prefix("https://github.com/")
        .unwrap_or("")
        .split('/')
        .collect();
    if parts.len() != 4
        || parts[2] != "pull"
        || parts[..2].iter().any(|s| {
            s.is_empty()
                || *s == "."
                || *s == ".."
                || s.starts_with('-')
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
        || parts[3].is_empty()
        || !parts[3].bytes().all(|b| b.is_ascii_digit())
        || parts[3].parse::<u64>().ok().is_none_or(|n| n == 0)
    {
        bail!("invalid GitHub PR URL; expected https://github.com/owner/repo/pull/number");
    }
    Ok(format!("{}/{}", parts[0], parts[1]))
}

pub fn pr_args<'a>(url: &'a str, command: &'a str, flags: &[&'a str]) -> Result<Vec<&'a str>> {
    repository(url)?;
    let mut args = vec!["pr", command];
    args.extend_from_slice(flags);
    args.extend(["--", url]);
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accepts_only_complete_pr_urls_and_uses_argument_separator() {
        let url = "https://github.com/owner/repo.name/pull/12";
        assert_eq!(repository(url).unwrap(), "owner/repo.name");
        assert_eq!(
            pr_args(url, "diff", &["--color", "never"]).unwrap(),
            vec!["pr", "diff", "--color", "never", "--", url]
        );
        for url in [
            "https://evilgithub.com/o/r/pull/1",
            "https://github.com.evil/o/r/pull/1",
            "prefix https://github.com/o/r/pull/1",
            "https://github.com//r/pull/1",
            "https://github.com/o/r/pull/no",
            "https://github.com/o/r/pull/1/files",
            "https://github.com/o/r/pull/0",
            "https://github.com/o/r/pull/1?x=y",
            "https://github.com/o/r/issues/1",
            "--help",
        ] {
            assert!(repository(url).is_err(), "{url}");
        }
    }
}
