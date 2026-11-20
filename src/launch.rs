use serde::Deserialize;

#[derive(Deserialize)]
struct Response {
    result: Panes,
}
#[derive(Deserialize)]
struct Panes {
    panes: Vec<Pane>,
}
#[derive(Deserialize)]
struct Pane {
    pane_id: String,
    tab_id: String,
    label: Option<String>,
    #[serde(default)]
    focused: bool,
}

impl Pane {
    fn is_linear(&self) -> bool {
        self.label.as_deref() == Some("Linear")
    }
}

/// Reuse within the caller's tab only. Never close on repeated launches.
pub fn decision(json: &str, caller_tab: Option<&str>) -> String {
    let Some((panes, tab)) = parse(json, caller_tab) else {
        return "OPEN".into();
    };
    focus_in_tab(&panes, &tab).unwrap_or_else(|| "OPEN".into())
}

/// Tab variant: reuse a Linear pane in the caller's tab, else switch to one in another tab
/// of the same workspace, else open a new tab. Never close on repeated launches, so a
/// supervised merge is not interrupted.
pub fn decision_tab(json: &str, caller_tab: Option<&str>) -> String {
    let Some((panes, tab)) = parse(json, caller_tab) else {
        return "OPEN".into();
    };
    if let Some(focus) = focus_in_tab(&panes, &tab) {
        return focus;
    }
    // Workspace comes from the `workspace:tab` id prefix; an unqualified id is unknown.
    let Some((workspace, _)) = tab.split_once(':').filter(|(w, _)| !w.is_empty()) else {
        return "OPEN".into();
    };
    panes
        .iter()
        .filter(|p| p.is_linear() && is_safe(&p.tab_id))
        .find(|p| p.tab_id.split_once(':').map(|(w, _)| w) == Some(workspace))
        .map_or_else(|| "OPEN".into(), |p| format!("SWITCHTAB {}", p.tab_id))
}

#[derive(Deserialize)]
struct Opened {
    result: OpenedResult,
}
#[derive(Deserialize)]
struct OpenedResult {
    plugin_pane: OpenedPane,
}
#[derive(Deserialize)]
struct OpenedPane {
    pane: OpenedTab,
}
#[derive(Deserialize)]
struct OpenedTab {
    tab_id: String,
}

/// The tab of a pane from `plugin pane open` output, so the launcher can switch to it:
/// `--focus` does not switch tabs. Empty when the output is unusable.
pub fn opened_tab(json: &str) -> String {
    serde_json::from_str::<Opened>(json)
        .map(|o| o.result.plugin_pane.pane.tab_id)
        .ok()
        .filter(|t| is_safe(t))
        .unwrap_or_default()
}

fn parse(json: &str, caller_tab: Option<&str>) -> Option<(Vec<Pane>, String)> {
    let panes = serde_json::from_str::<Response>(json).ok()?.result.panes;
    let tab = caller_tab
        .or_else(|| panes.iter().find(|p| p.focused).map(|p| p.tab_id.as_str()))
        .filter(|s| !s.is_empty())?
        .to_owned();
    Some((panes, tab))
}

fn focus_in_tab(panes: &[Pane], tab: &str) -> Option<String> {
    panes
        .iter()
        .find(|p| p.tab_id == tab && p.is_linear())
        .filter(|p| is_safe(&p.pane_id))
        .map(|p| format!("FOCUS {}", p.pane_id))
}

/// An ID is safe to pass as an argument: non-empty, no leading `-`, `[A-Za-z0-9_:.-]` only.
fn is_safe(id: &str) -> bool {
    !id.is_empty()
        && !id.starts_with('-')
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_:.-".contains(c))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_pane_is_focused_not_duplicated_or_closed() {
        let input = r#"{"result":{"panes":[{"pane_id":"w1:p1","tab_id":"w1:t1","focused":true,"label":"Linear"}]}}"#;
        assert_eq!(decision(input, None), "FOCUS w1:p1");
        assert_eq!(decision(input, Some("w1:t2")), "OPEN");
        assert_eq!(decision("invalid", None), "OPEN");
        assert_eq!(decision(&input.replace("w1:p1", "--bad"), None), "OPEN");
        assert_eq!(decision(r#"{"result":{"panes":[]}}"#, None), "OPEN");
    }

    fn list(panes: &[(&str, &str, bool, &str)]) -> String {
        let panes: Vec<_> = panes
            .iter()
            .map(|(id, tab, focused, label)| {
                format!(
                    r#"{{"pane_id":"{id}","tab_id":"{tab}","focused":{focused},"label":"{label}"}}"#
                )
            })
            .collect();
        format!(r#"{{"result":{{"panes":[{}]}}}}"#, panes.join(","))
    }

    #[test]
    fn tab_launch_reuses_this_workspace_and_never_closes() {
        let here = list(&[("w1:p1", "w1:t1", true, "Linear")]);
        assert_eq!(decision_tab(&here, None), "FOCUS w1:p1");

        let elsewhere = list(&[
            ("w1:p1", "w1:t1", true, "zsh"),
            ("w1:p2", "w1:t2", false, "Linear"),
        ]);
        assert_eq!(decision_tab(&elsewhere, None), "SWITCHTAB w1:t2");
        assert_eq!(decision_tab(&elsewhere, Some("w1:t1")), "SWITCHTAB w1:t2");

        let other_workspace = list(&[
            ("w1:p1", "w1:t1", true, "zsh"),
            ("w2:p2", "w2:t2", false, "Linear"),
        ]);
        assert_eq!(decision_tab(&other_workspace, None), "OPEN");
    }

    #[test]
    fn opened_tab_is_validated() {
        let opened = r#"{"id":"x","result":{"type":"plugin_pane_opened","plugin_pane":{"plugin_id":"herdr-linear","entrypoint":"issues","pane":{"pane_id":"w1:p9","tab_id":"w1:t9"}}}}"#;
        assert_eq!(opened_tab(opened), "w1:t9");
        assert_eq!(opened_tab(&opened.replace("w1:t9", "-w1:t9")), "");
        assert_eq!(opened_tab("invalid"), "");
    }

    #[test]
    fn tab_launch_opens_when_unsure() {
        assert_eq!(decision_tab("invalid", None), "OPEN");
        let none = list(&[("w1:p1", "w1:t1", true, "zsh")]);
        assert_eq!(decision_tab(&none, None), "OPEN");
        let unqualified = list(&[("p1", "t1", true, "zsh"), ("p2", "t2", false, "Linear")]);
        assert_eq!(decision_tab(&unqualified, None), "OPEN");
        let unsafe_tab = list(&[
            ("w1:p1", "w1:t1", true, "zsh"),
            ("w1:p2", "-w1:t2", false, "Linear"),
        ]);
        assert_eq!(decision_tab(&unsafe_tab, None), "OPEN");
    }
}
