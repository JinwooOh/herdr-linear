use std::{
    collections::{HashMap, HashSet},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use serde::{de::DeserializeOwned, Deserialize};
use serde_json::{json, Value};

const ENDPOINT: &str = "https://api.linear.app/graphql";
const QUERY: &str = r#"
query($after: String) {
  viewer {
    assignedIssues(
      first: 100
      after: $after
      orderBy: updatedAt
      filter: { state: { type: { nin: ["completed", "canceled"] } } }
    ) {
      pageInfo { hasNextPage endCursor }
      nodes {
        id identifier title url priority branchName updatedAt
        team { id }
        state { id name type }
        attachments { nodes { title url sourceType } }
      }
    }
  }
}
"#;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Issue {
    pub id: String,
    pub identifier: String,
    pub title: String,
    pub url: String,
    pub priority: u8,
    #[serde(default)]
    pub branch_name: String,
    pub updated_at: String,
    pub team: Team,
    pub state: State,
    pub attachments: Attachments,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Team {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct State {
    pub id: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub position: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Attachments {
    pub nodes: Vec<Attachment>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Attachment {
    pub title: Option<String>,
    pub url: String,
    pub source_type: Option<String>,
}

impl Issue {
    pub fn is_in_review(&self) -> bool {
        self.state.kind == "started" && self.state.name.eq_ignore_ascii_case("in review")
    }

    pub fn is_open(&self) -> bool {
        !matches!(
            self.state.kind.as_str(),
            "completed" | "canceled" | "duplicate"
        )
    }

    pub fn pull_requests(&self) -> impl Iterator<Item = &Attachment> {
        self.attachments
            .nodes
            .iter()
            .filter(|a| crate::github::repository(&a.url).is_ok())
    }

    pub fn priority_label(&self) -> &'static str {
        match self.priority {
            1 => "Urgent",
            2 => "High",
            3 => "Medium",
            4 => "Low",
            _ => "No priority",
        }
    }
}

pub fn api_key() -> Result<String> {
    if let Ok(key) = std::env::var("LINEAR_API_KEY") {
        if !key.trim().is_empty() {
            return Ok(key.trim().to_string());
        }
    }
    if let Ok(dir) = std::env::var("HERDR_PLUGIN_CONFIG_DIR") {
        if let Ok(key) = std::fs::read_to_string(std::path::Path::new(&dir).join("api_key")) {
            if !key.trim().is_empty() {
                return Ok(key.trim().to_string());
            }
        }
    }
    bail!("no Linear API key: set LINEAR_API_KEY or write it to $HERDR_PLUGIN_CONFIG_DIR/api_key")
}

#[derive(Deserialize)]
struct Response<T> {
    data: Option<T>,
    errors: Option<Vec<GraphQlError>>,
}
#[derive(Deserialize)]
struct GraphQlError {
    message: String,
}

fn decode<T: DeserializeOwned>(response: Response<T>) -> Result<T> {
    if let Some(errors) = response.errors.filter(|errors| !errors.is_empty()) {
        bail!(
            "Linear returned errors: {}",
            errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ")
        );
    }
    response.data.context("Linear response had no data")
}

fn request<T: DeserializeOwned>(key: &str, query: &str, variables: Value) -> Result<T> {
    let agent = ureq::AgentBuilder::new()
        .timeout(Duration::from_secs(20))
        .build();
    let response = agent
        .post(ENDPOINT)
        .set("Authorization", key)
        .send_json(json!({"query": query, "variables": variables}));
    let response: Response<T> = http_response(response)?
        .into_json()
        .context("could not parse Linear response")?;
    decode(response)
}

fn http_response(
    result: std::result::Result<ureq::Response, ureq::Error>,
) -> Result<ureq::Response> {
    match result {
        Ok(response) => Ok(response),
        Err(ureq::Error::Status(code, response)) => {
            use std::io::Read;
            let mut body = String::new();
            let _ = response.into_reader().take(65536).read_to_string(&mut body);
            Err(status_error(code, &body))
        }
        Err(error) => Err(anyhow::anyhow!("{UNKNOWN_OUTCOME}: {error}")),
    }
}

/// Transport failures (timeouts, resets) cannot tell whether a mutation was applied.
pub const UNKNOWN_OUTCOME: &str = "Linear transport error; request outcome unknown";

fn status_error(code: u16, body: &str) -> anyhow::Error {
    let messages = serde_json::from_str::<Value>(body)
        .ok()
        .and_then(|v| v.get("errors").and_then(Value::as_array).cloned())
        .unwrap_or_default()
        .iter()
        .filter_map(|e| e.get("message").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("; ");
    let action = match code {
        401 => "Authentication failed; check this Account's API key.",
        429 => "Rate limited; wait before refreshing again.",
        _ => "Linear request rejected.",
    };
    if messages.is_empty() {
        anyhow::anyhow!("Linear HTTP {code}: {action}")
    } else {
        anyhow::anyhow!("Linear HTTP {code}: {action} {messages}")
    }
}

fn sort_states(states: &mut [State]) {
    fn rank(kind: &str) -> u8 {
        match kind {
            "triage" => 0,
            "backlog" => 1,
            "unstarted" => 2,
            "started" => 3,
            "completed" => 4,
            "canceled" => 5,
            "duplicate" => 6,
            _ => 7,
        }
    }
    states.sort_by(|a, b| {
        rank(&a.kind)
            .cmp(&rank(&b.kind))
            .then(a.position.total_cmp(&b.position))
            .then(a.id.cmp(&b.id))
    });
}

#[derive(Deserialize)]
struct Data {
    viewer: Viewer,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Viewer {
    assigned_issues: Page,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Page {
    nodes: Vec<Issue>,
    page_info: PageInfo,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageInfo {
    has_next_page: bool,
    end_cursor: Option<String>,
}

pub fn fetch_issues(key: &str) -> Result<Vec<Issue>> {
    collect_pages(|cursor| {
        let data: Data = request(key, QUERY, json!({"after": cursor}))?;
        Ok(data.viewer.assigned_issues)
    })
}

fn collect_pages(mut fetch: impl FnMut(Option<&str>) -> Result<Page>) -> Result<Vec<Issue>> {
    let mut cursor: Option<String> = None;
    let mut cursors = HashSet::new();
    let mut issues: HashMap<String, Issue> = HashMap::new();
    loop {
        let page = fetch(cursor.as_deref()).context("could not load assigned issue page")?;
        for issue in page.nodes {
            if issues
                .get(&issue.id)
                .is_none_or(|old| old.updated_at <= issue.updated_at)
            {
                issues.insert(issue.id.clone(), issue);
            }
        }
        if !page.page_info.has_next_page {
            break;
        }
        let next = page
            .page_info
            .end_cursor
            .filter(|c| !c.is_empty())
            .context("Linear pagination returned no cursor")?;
        if !cursors.insert(next.clone()) {
            bail!("Linear pagination repeated a cursor");
        }
        cursor = Some(next);
    }
    let mut issues: Vec<_> = issues.into_values().filter(Issue::is_open).collect();
    issues.sort_by(|a, b| b.updated_at.cmp(&a.updated_at).then(a.id.cmp(&b.id)));
    Ok(issues)
}

pub fn fetch_states(key: &str, team_id: &str) -> Result<Vec<State>> {
    #[derive(Deserialize)]
    struct States {
        nodes: Vec<State>,
    }
    #[derive(Deserialize)]
    struct StateTeam {
        states: States,
    }
    #[derive(Deserialize)]
    struct StateData {
        team: Option<StateTeam>,
    }
    let data: StateData = request(
        key,
        "query($id: String!) { team(id: $id) { states { nodes { id name type position } } } }",
        json!({"id": team_id}),
    )?;
    let mut states = data.team.context("Linear team was not found")?.states.nodes;
    sort_states(&mut states);
    Ok(states)
}

#[derive(Deserialize)]
struct UpdatedIssue {
    state: State,
}
#[derive(Deserialize)]
struct Payload {
    success: bool,
    issue: Option<UpdatedIssue>,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MutationData {
    issue_update: Payload,
}

pub fn change_state(key: &str, issue_id: &str, state_id: &str) -> Result<State> {
    let data: MutationData = request(key,
        "mutation($id: String!, $state: String!) { issueUpdate(id: $id, input: {stateId: $state}) { success issue { state { id name type } } } }",
        json!({"id":issue_id,"state":state_id}))?;
    updated_state(data)
}

fn updated_state(data: MutationData) -> Result<State> {
    if !data.issue_update.success {
        bail!("Linear did not update the issue state");
    }
    Ok(data
        .issue_update
        .issue
        .context("Linear mutation returned no issue")?
        .state)
}

#[cfg(test)]
mod tests;
