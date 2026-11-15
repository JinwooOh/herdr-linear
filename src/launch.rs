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

/// Reuse within the caller's tab only. Never close on repeated launches.
pub fn decision(json: &str, caller_tab: Option<&str>) -> String {
    let Ok(response) = serde_json::from_str::<Response>(json) else {
        return "OPEN".into();
    };
    let panes = response.result.panes;
    let tab = caller_tab.or_else(|| panes.iter().find(|p| p.focused).map(|p| p.tab_id.as_str()));
    let Some(tab) = tab.filter(|s| !s.is_empty()) else {
        return "OPEN".into();
    };
    let pane = panes
        .iter()
        .find(|p| p.tab_id == tab && p.label.as_deref() == Some("Linear"));
    match pane {
        Some(p)
            if !p.pane_id.is_empty()
                && !p.pane_id.starts_with('-')
                && p.pane_id
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || "_:.-".contains(c)) =>
        {
            format!("FOCUS {}", p.pane_id)
        }
        _ => "OPEN".into(),
    }
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
}
