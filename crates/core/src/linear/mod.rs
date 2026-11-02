//! Linear importer: a small blocking GraphQL client (`ureq` + `serde_json`)
//! and the pure JSON → message mapping in [`map`].
//!
//! The endpoint is `https://api.linear.app/graphql`; the hidden
//! `LINEAR_API_URL` env var points it elsewhere (tests use a mock server).
//!
//! The import itself (dedupe, create, `--existing update`) is in [`import`].

pub mod import;
pub mod map;

use crate::error::{Error, Result};
use serde_json::{Value, json};
use std::time::Duration;

const DEFAULT_URL: &str = "https://api.linear.app/graphql";
/// Retries after a rate-limit response before giving up.
const RATE_LIMIT_RETRIES: u32 = 3;
const MAX_WAIT_SECS: u64 = 60;

/// Called with the wait in seconds before retrying a rate-limited request.
pub type RateLimitHook = Box<dyn Fn(u64) + Send + Sync>;

pub struct Client {
    agent: ureq::Agent,
    url: String,
    key: String,
    on_rate_limit: Option<RateLimitHook>,
}

fn api_error(kind: &str, msg: impl Into<String>) -> Error {
    Error::usage(kind).with("Message", msg)
}

impl Client {
    pub fn new(key: &str) -> Self {
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(60)))
            .build()
            .into();
        Client {
            agent,
            url: std::env::var("LINEAR_API_URL").unwrap_or_else(|_| DEFAULT_URL.into()),
            key: key.trim().to_string(),
            on_rate_limit: None,
        }
    }

    /// Be told about rate-limit waits (core never prints; the CLI reports
    /// them on stderr).
    pub fn on_rate_limit(mut self, hook: impl Fn(u64) + Send + Sync + 'static) -> Self {
        self.on_rate_limit = Some(Box::new(hook));
        self
    }

    /// Run one GraphQL query and return its `data`. Honors 429 / RATELIMITED
    /// with Retry-After (a few times), then fails.
    pub fn query(&self, query: &str, vars: Value) -> Result<Value> {
        let payload = json!({ "query": query, "variables": vars }).to_string();
        let mut attempt = 0;
        loop {
            let mut resp = self
                .agent
                .post(&self.url)
                .header("Authorization", &self.key)
                .header("Content-Type", "application/json")
                .send(&payload)
                .map_err(|e| api_error("linear-unreachable", e.to_string()))?;
            let status = resp.status().as_u16();
            let retry_after = resp
                .headers()
                .get("retry-after")
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.trim().parse::<u64>().ok());
            let text = resp
                .body_mut()
                .with_config()
                .limit(64 * 1024 * 1024)
                .read_to_string()
                .unwrap_or_default();
            let parsed: Option<Value> = serde_json::from_str(&text).ok();
            let code = parsed
                .as_ref()
                .and_then(|v| v["errors"][0]["extensions"]["code"].as_str())
                .unwrap_or("")
                .to_string();
            let message = parsed
                .as_ref()
                .and_then(|v| v["errors"][0]["message"].as_str())
                .map(str::to_string)
                .unwrap_or_else(|| text.chars().take(200).collect());

            if status == 429 || code.eq_ignore_ascii_case("RATELIMITED") {
                if attempt >= RATE_LIMIT_RETRIES {
                    return Err(api_error("linear-rate-limited", message));
                }
                let wait = retry_after.unwrap_or(1 << attempt).min(MAX_WAIT_SECS);
                if let Some(hook) = &self.on_rate_limit {
                    hook(wait);
                }
                std::thread::sleep(Duration::from_secs(wait));
                attempt += 1;
                continue;
            }
            if status == 401 || status == 403 || code.eq_ignore_ascii_case("AUTHENTICATION_ERROR") {
                return Err(Error::usage("linear-auth").with("Message", message).with(
                    "Hint",
                    "check --api-key / LINEAR_API_KEY (a personal API key)",
                ));
            }
            if !(200..300).contains(&status) {
                return Err(api_error("linear-api", message).with("Status", status.to_string()));
            }
            let v = parsed.ok_or_else(|| api_error("linear-api", "response is not JSON"))?;
            if v["errors"].is_array() && v["data"].is_null() {
                return Err(api_error("linear-api", message));
            }
            return Ok(v["data"].clone());
        }
    }

    /// Fetch every node of a top-level connection (`projects`, `teams`, …).
    fn all_nodes(&self, query: &str, field: &str, vars: Value) -> Result<Vec<Value>> {
        let mut out = Vec::new();
        let mut after: Option<String> = None;
        loop {
            let mut v = vars.clone();
            v["after"] = json!(after);
            let data = self.query(query, v)?;
            let conn = &data[field];
            out.extend(conn["nodes"].as_array().cloned().unwrap_or_default());
            match next_cursor(conn) {
                Some(c) => after = Some(c),
                None => return Ok(out),
            }
        }
    }

    pub fn viewer(&self) -> Result<Viewer> {
        let d = self.query(VIEWER, json!({}))?;
        Ok(Viewer {
            name: str_of(&d["viewer"]["name"]),
            email: str_of(&d["viewer"]["email"]),
            org: str_of(&d["viewer"]["organization"]["name"]),
        })
    }

    pub fn projects(&self) -> Result<Vec<Source>> {
        Ok(self
            .all_nodes(PROJECTS, "projects", json!({}))?
            .iter()
            .map(|n| Source {
                kind: SourceKind::Project,
                id: str_of(&n["id"]),
                key: str_of(&n["slugId"]),
                name: str_of(&n["name"]),
                closed: matches!(
                    n["state"].as_str().unwrap_or("").to_lowercase().as_str(),
                    "completed" | "canceled" | "cancelled"
                ),
            })
            .collect())
    }

    pub fn teams(&self) -> Result<Vec<Source>> {
        Ok(self
            .all_nodes(TEAMS, "teams", json!({}))?
            .iter()
            .map(|n| Source {
                kind: SourceKind::Team,
                id: str_of(&n["id"]),
                key: str_of(&n["key"]),
                name: str_of(&n["name"]),
                closed: false,
            })
            .collect())
    }

    /// Cheap pre-pass: (uuid, identifier) of every issue in scope.
    pub fn issue_ids(&self, src: &Source) -> Result<Vec<(String, String)>> {
        Ok(self
            .all_nodes(ISSUE_IDS, "issues", json!({ "filter": src.filter() }))?
            .iter()
            .map(|n| (str_of(&n["id"]), str_of(&n["identifier"])))
            .collect())
    }

    /// One page of full issues; nested connections are completed so every
    /// node is self-contained. Returns (nodes, next cursor).
    pub fn issue_page(
        &self,
        src: &Source,
        after: Option<&str>,
    ) -> Result<(Vec<Value>, Option<String>)> {
        let d = self.query(ISSUES, json!({ "filter": src.filter(), "after": after }))?;
        let conn = &d["issues"];
        let mut nodes = conn["nodes"].as_array().cloned().unwrap_or_default();
        for n in &mut nodes {
            self.complete_nested(n)?;
        }
        Ok((nodes, next_cursor(conn)))
    }

    /// Page through any nested connection that was cut off.
    fn complete_nested(&self, issue: &mut Value) -> Result<()> {
        let id = str_of(&issue["id"]);
        for (field, selection) in NESTED {
            let mut cursor = next_cursor(&issue[*field]);
            while let Some(after) = cursor {
                // GraphQL rejects unused fragments: include only what's used.
                let mut frags = String::new();
                if selection.contains("...Ref") {
                    frags.push_str(FRAG_REF);
                }
                if selection.contains("...C") {
                    frags.push_str(FRAG_C);
                }
                let q = format!(
                    "query($id: String!, $after: String) {{ issue(id: $id) {{ {field}(first: 100, after: $after) {{ nodes {{ {selection} }} pageInfo {{ hasNextPage endCursor }} }} }} }} {frags}"
                );
                let d = self.query(&q, json!({ "id": id, "after": after }))?;
                let conn = &d["issue"][*field];
                let more = conn["nodes"].as_array().cloned().unwrap_or_default();
                if let Some(arr) = issue[*field]["nodes"].as_array_mut() {
                    arr.extend(more);
                }
                cursor = next_cursor(conn);
            }
        }
        Ok(())
    }
}

fn next_cursor(conn: &Value) -> Option<String> {
    (conn["pageInfo"]["hasNextPage"].as_bool() == Some(true))
        .then(|| conn["pageInfo"]["endCursor"].as_str().map(str::to_string))
        .flatten()
}

pub fn str_of(v: &Value) -> String {
    v.as_str().unwrap_or("").to_string()
}

pub struct Viewer {
    pub name: String,
    pub email: String,
    pub org: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceKind {
    Project,
    Team,
}

/// What is being imported: one Linear project or team.
#[derive(Debug, Clone)]
pub struct Source {
    pub kind: SourceKind,
    pub id: String,
    /// Project slugId or team key.
    pub key: String,
    pub name: String,
    /// Project completed / cancelled (hidden in the picker unless --all).
    pub closed: bool,
}

impl Source {
    fn filter(&self) -> Value {
        match self.kind {
            SourceKind::Project => json!({ "project": { "id": { "eq": self.id } } }),
            SourceKind::Team => json!({ "team": { "id": { "eq": self.id } } }),
        }
    }

    /// Does `query` name this source? Project: uuid, slugId, URL slug or
    /// name; team: key, uuid or name. Case-insensitive.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().trim_end_matches('/');
        let q = q.rsplit('/').next().unwrap_or(q).to_lowercase();
        let eq = |s: &str| !s.is_empty() && s.to_lowercase() == q;
        eq(&self.id)
            || eq(&self.key)
            || eq(&self.name)
            || (self.kind == SourceKind::Project
                && !self.key.is_empty()
                && q.ends_with(&format!("-{}", self.key.to_lowercase())))
    }

    pub fn label(&self) -> String {
        match self.kind {
            SourceKind::Project => self.name.clone(),
            SourceKind::Team => format!("{} ({})", self.name, self.key),
        }
    }
}

// ---------------------------------------------------------------- queries

const VIEWER: &str = "query { viewer { name email organization { name } } }";

const PROJECTS: &str = "query($after: String) { projects(first: 100, after: $after) { nodes { id name slugId state } pageInfo { hasNextPage endCursor } } }";

const TEAMS: &str = "query($after: String) { teams(first: 100, after: $after) { nodes { id key name } pageInfo { hasNextPage endCursor } } }";

const ISSUE_IDS: &str = "query($filter: IssueFilter, $after: String) { issues(first: 250, after: $after, filter: $filter) { nodes { id identifier } pageInfo { hasNextPage endCursor } } }";

/// Nested connections of an issue and their node selections. Each is
/// fetched 20 at a time in the page query and completed per issue after.
const NESTED: &[(&str, &str)] = &[
    ("labels", "name"),
    ("subscribers", "name email"),
    ("attachments", "title url"),
    ("relations", "type relatedIssue { ...Ref }"),
    ("inverseRelations", "type issue { ...Ref }"),
    ("comments", "...C"),
];

const FRAG_REF: &str = " fragment Ref on Issue { id createdAt project { id } team { id } }";
const FRAG_C: &str = " fragment C on Comment { id body createdAt editedAt user { name email } externalUser { name email } botActor { name } parent { id createdAt } }";

const ISSUES: &str = "query($filter: IssueFilter, $after: String) { issues(first: 25, after: $after, filter: $filter, orderBy: createdAt) { nodes { id identifier url title description createdAt priority dueDate creator { name email } assignee { name email } state { name type } project { id name } team { id } parent { ...Ref } labels(first: 20) { nodes { name } pageInfo { hasNextPage endCursor } } subscribers(first: 20) { nodes { name email } pageInfo { hasNextPage endCursor } } attachments(first: 20) { nodes { title url } pageInfo { hasNextPage endCursor } } relations(first: 20) { nodes { type relatedIssue { ...Ref } } pageInfo { hasNextPage endCursor } } inverseRelations(first: 20) { nodes { type issue { ...Ref } } pageInfo { hasNextPage endCursor } } comments(first: 20) { nodes { ...C } pageInfo { hasNextPage endCursor } } } pageInfo { hasNextPage endCursor } } } fragment Ref on Issue { id createdAt project { id } team { id } } fragment C on Comment { id body createdAt editedAt user { name email } externalUser { name email } botActor { name } parent { id createdAt } }";

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> Source {
        Source {
            kind: SourceKind::Project,
            id: "9f1c-uuid".into(),
            key: "a1b2c3d4e5f6".into(),
            name: "Billing Revamp".into(),
            closed: false,
        }
    }

    #[test]
    fn source_matching() {
        let p = project();
        for q in [
            "billing revamp",
            "9F1C-UUID",
            "a1b2c3d4e5f6",
            "billing-revamp-a1b2c3d4e5f6",
            "https://linear.app/acme/project/billing-revamp-a1b2c3d4e5f6/",
        ] {
            assert!(p.matches(q), "{q}");
        }
        assert!(!p.matches("billing"));
    }

    #[test]
    fn fragments_in_sync() {
        assert!(ISSUES.ends_with(&format!("}}{FRAG_REF}{FRAG_C}")));
    }
}
