//! Listing: filtering, full-text search, sorting and paging.
//!
//! A [`ListQuery`] holds the user's criteria as typed
//! (WHO, issue and project references). [`Filter::resolve`] turns them into
//! concrete values once; [`list`] then streams issues in path order through
//! the filter on the thread pool, stopping early when no sort needs the
//! full set.
//!
//! Filter tokens parse with [`tokens`] and fold
//! into a [`ListQuery`] with [`ListQuery::apply`], so the token parser, the
//! CLI flags and the GUI share one evaluation path.

use crate::error::{EXIT_NOT_FOUND, Error, Result};
use crate::identity::Identity;
use crate::model::{self, Priority, State, labels_of, list_of, priority_of, state_of};
use crate::search::{Matches, Terms};
use crate::store::mbox::{self, Msg};
use crate::store::{self, IssueRef, Project, Store};
use chrono::{DateTime, FixedOffset};
use rayon::prelude::*;
use std::borrow::Cow;
use std::cmp::Reverse;
use std::collections::HashMap;

mod memory;
pub use memory::{
    Blockers, Group, GroupKey, Grouping, Tier, group, instant_match, sort_summaries, term_tier,
};

/// Sort orders other than path order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sort {
    /// in-progress, in-review, todo, backlog, closed; then priority; then
    /// newest.
    State,
    /// Newest first.
    Created,
    /// Most recent message first.
    Updated,
    /// Most urgent first, then newest.
    Priority,
    /// Soonest due first (undated last), then the state order.
    Due,
}

/// What `issue list` asks for. `Default` is every open issue, unlimited.
#[derive(Debug, Clone, Default)]
pub struct ListQuery {
    /// Project reference; only its issues.
    pub project: Option<String>,
    /// Only issues outside any project (ignored when `project` is set).
    pub no_project: bool,
    /// Only these states (closed ones included); empty: see `all`.
    pub states: Vec<State>,
    /// Include closed issues.
    pub all: bool,
    /// Has all these labels.
    pub labels: Vec<String>,
    /// WHO patterns.
    pub assignee: Option<String>,
    pub author: Option<String>,
    pub watching: Option<String>,
    /// Only issues with a `Sender:` (written by an agent).
    pub agent: bool,
    /// Only children of this issue reference.
    pub parent: Option<String>,
    /// Only todo issues whose blockers are all closed.
    pub ready: bool,
    /// Only issues with at least one blocker in an open state.
    pub blocked: bool,
    /// Only these priorities (`None`: no priority); empty: any.
    pub priorities: Vec<Option<Priority>>,
    /// Date input: only issues due before it.
    pub due_before: Option<String>,
    /// Date input: only issues due after it.
    pub due_after: Option<String>,
    /// Only issues past their `Expires:` date.
    pub expired: bool,
    /// Full-text terms, all of which must match. Non-empty search includes
    /// closed issues unless `states` narrows it.
    pub search: Vec<String>,
    /// Treat `search` terms as regexes.
    pub regex: bool,
    /// `None`: path order (and early stop).
    pub sort: Option<Sort>,
    /// Maximum rows; 0 = unlimited.
    pub limit: usize,
    pub offset: usize,
    /// Fill [`Row::stats`] for every returned row.
    pub stats: bool,
}

/// Comment count and the date of the newest message.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stats {
    pub comments: usize,
    pub updated: Option<DateTime<FixedOffset>>,
}

/// One listed issue.
#[derive(Debug, Clone)]
pub struct Row {
    pub issue: IssueRef,
    pub head: Msg,
    /// Where `search` matched.
    pub matches: Option<Matches>,
    pub stats: Option<Stats>,
}

/// A page of results.
#[derive(Debug, Clone)]
pub struct Page {
    pub rows: Vec<Row>,
    /// The `offset` of the next page, when more results may exist.
    pub next_offset: Option<usize>,
}

/// A [`ListQuery`] with every reference resolved.
#[derive(Debug, Clone)]
pub struct Filter {
    pub project: Option<Project>,
    pub no_project: bool,
    pub states: Vec<State>,
    /// Closed issues pass (`all`, or a search).
    pub include_closed: bool,
    pub labels: Vec<String>,
    pub assignee: Option<String>,
    pub author: Option<String>,
    pub watching: Option<String>,
    pub agent: bool,
    /// Parent's Message-ID.
    pub parent: Option<String>,
    pub ready: bool,
    pub blocked: bool,
    pub priorities: Vec<Option<Priority>>,
    /// Unix-time bounds on `Reply-By` (exclusive).
    pub due_before: Option<i64>,
    pub due_after: Option<i64>,
    pub expired: bool,
    pub terms: Option<Terms>,
    /// The raw free-text terms, lowercased (the in-memory instant tier,
    /// [`instant_match`]).
    pub text: Vec<String>,
    /// Unix time `expired` compares against.
    pub now: i64,
}

/// `s` lowercased, without allocating when it already is.
pub fn lower(s: &str) -> Cow<'_, str> {
    if s.chars().any(char::is_uppercase) {
        Cow::Owned(s.to_lowercase())
    } else {
        Cow::Borrowed(s)
    }
}

/// Case-insensitive substring test for an already-lowercase `needle`.
/// ASCII text (the common case) is compared in place; anything else falls
/// back to `to_lowercase`.
pub fn contains_lower(hay: &str, needle: &str) -> bool {
    if needle.is_empty() {
        return true;
    }
    if hay.is_ascii() && needle.is_ascii() {
        let n = needle.as_bytes();
        return hay
            .as_bytes()
            .windows(n.len())
            .any(|w| w.eq_ignore_ascii_case(n));
    }
    hay.to_lowercase().contains(needle)
}

impl Filter {
    pub fn resolve(store: &Store, me: &Identity, q: &ListQuery) -> Result<Filter> {
        let project = q
            .project
            .as_deref()
            .map(|p| resolve_project(store, p))
            .transpose()?;
        let now = model::now();
        let due = |d: &Option<String>| -> Result<Option<i64>> {
            d.as_deref()
                .map(|d| Ok(crate::dates::parse(d, now)?.timestamp()))
                .transpose()
        };
        let parent = q
            .parent
            .as_deref()
            .map(|p| crate::ops::issue_message_id(store, p))
            .transpose()?;
        let pattern = |w: &Option<String>| w.as_deref().map(|w| me.who_pattern(w)).transpose();
        let assignee = pattern(&q.assignee)?;
        let author = pattern(&q.author)?;
        let watching = pattern(&q.watching)?;
        let terms = (!q.search.is_empty())
            .then(|| Terms::new(&q.search, q.regex))
            .transpose()?;
        Ok(Filter {
            project,
            no_project: q.no_project,
            states: q.states.clone(),
            include_closed: q.all || terms.is_some(),
            labels: model::normalize_labels(q.labels.iter().map(String::as_str)),
            assignee,
            author,
            watching,
            agent: q.agent,
            parent,
            ready: q.ready,
            blocked: q.blocked,
            priorities: q.priorities.clone(),
            due_before: due(&q.due_before)?,
            due_after: due(&q.due_after)?,
            expired: q.expired,
            terms,
            text: q.search.iter().map(|t| t.to_lowercase()).collect(),
            now: now.timestamp(),
        })
    }

    /// An issue in the project with this ID (`None`: outside any) is in
    /// scope.
    fn in_scope(&self, project: Option<&str>) -> bool {
        match (&self.project, self.no_project) {
            (Some(p), _) => project == Some(p.id.as_str()),
            (None, true) => project.is_none(),
            _ => true,
        }
    }

    /// `is:ready` / `is:blocked`, given whether the issue has an open
    /// blocker.
    fn passes_blocked(&self, blocked: bool) -> bool {
        (!self.ready || !blocked) && (!self.blocked || blocked)
    }

    /// Every header criterion except `ready`'s blocker check (which needs
    /// other issues) and the full-text terms.
    pub(crate) fn matches_fields<H: HeadFields>(&self, head: &H) -> bool {
        let st = head.state();
        if !self.states.is_empty() {
            if !self.states.contains(&st) {
                return false;
            }
        } else if !self.include_closed && !st.is_open() {
            return false;
        }
        if self.ready && st != State::Todo {
            return false;
        }
        if !self.priorities.is_empty() && !self.priorities.contains(&head.priority()) {
            return false;
        }
        if self.due_before.is_some() || self.due_after.is_some() {
            let Some(d) = head.due() else {
                return false;
            };
            if self.due_before.is_some_and(|b| d >= b) || self.due_after.is_some_and(|a| d <= a) {
                return false;
            }
        }
        if !self.labels.is_empty() {
            let have = head.labels();
            if !self.labels.iter().all(|l| have.contains(l)) {
                return false;
            }
        }
        if let Some(w) = &self.assignee
            && !head.assignee().is_some_and(|v| contains_lower(v, w))
        {
            return false;
        }
        if let Some(w) = &self.author
            && !head.author().is_some_and(|v| contains_lower(v, w))
        {
            return false;
        }
        if let Some(w) = &self.watching
            && !head.watchers().iter().any(|v| contains_lower(v, w))
        {
            return false;
        }
        if self.agent && !head.has_sender() {
            return false;
        }
        if let Some(p) = &self.parent
            && head.parent() != Some(p.as_str())
        {
            return false;
        }
        if self.expired && head.expires().is_none_or(|d| d >= self.now) {
            return false;
        }
        true
    }
}

/// The message #1 fields a [`Filter`] and the sort orders look at, so
/// parsed heads ([`Msg`]) and in-memory summaries share one evaluation.
pub(crate) trait HeadFields {
    fn state(&self) -> State;
    fn priority(&self) -> Option<Priority>;
    /// Unix time of `Date` (0 without one).
    fn created(&self) -> i64;
    /// Unix time of `Reply-By`.
    fn due(&self) -> Option<i64>;
    /// Unix time of `Expires`.
    fn expires(&self) -> Option<i64>;
    fn labels(&self) -> Cow<'_, [String]>;
    fn assignee(&self) -> Option<&str>;
    fn author(&self) -> Option<&str>;
    fn watchers(&self) -> Cow<'_, [String]>;
    fn has_sender(&self) -> bool;
    fn parent(&self) -> Option<&str>;
}

fn ts(m: &Msg, header: &str) -> Option<i64> {
    m.get(header)
        .and_then(mbox::parse_date)
        .map(|d| d.timestamp())
}

impl HeadFields for Msg {
    fn state(&self) -> State {
        state_of(self)
    }
    fn priority(&self) -> Option<Priority> {
        priority_of(self)
    }
    fn created(&self) -> i64 {
        self.date().map_or(0, |d| d.timestamp())
    }
    fn due(&self) -> Option<i64> {
        ts(self, "Reply-By")
    }
    fn expires(&self) -> Option<i64> {
        ts(self, "Expires")
    }
    fn labels(&self) -> Cow<'_, [String]> {
        Cow::Owned(labels_of(self))
    }
    fn assignee(&self) -> Option<&str> {
        self.get("To")
    }
    fn author(&self) -> Option<&str> {
        self.get("From")
    }
    fn watchers(&self) -> Cow<'_, [String]> {
        Cow::Owned(list_of(self, "Cc"))
    }
    fn has_sender(&self) -> bool {
        self.has("Sender")
    }
    fn parent(&self) -> Option<&str> {
        self.get("X-Parent")
    }
}

/// A project for a list filter: an ID reference as everywhere, or else a
/// slug (exact, or a unique prefix), so `project:auth` works.
fn resolve_project(store: &Store, q: &str) -> Result<Project> {
    match store.resolve_project(q) {
        Err(e) if e.code == EXIT_NOT_FOUND => {
            let want = q.trim().to_lowercase();
            let mut hits = store::matching(store.projects()?, |p| p.slug.as_str(), &want);
            match hits.len() {
                0 => Err(e),
                1 => Ok(hits.remove(0)),
                _ => Err(Error::ambiguous(hits.into_iter().map(|p| p.id).collect())),
            }
        }
        r => r,
    }
}

// ---------------------------------------------------------------- tokens

/// One filter token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Token {
    /// `state:S`; repeated tokens are alternatives.
    State(State),
    /// `p:P` / `priority:P` (`none` for no priority); alternatives.
    Priority(Option<Priority>),
    /// `label:L`; all must match.
    Label(String),
    /// `@WHO` / `assignee:WHO`.
    Assignee(String),
    /// `project:P` (ID prefix or slug).
    Project(String),
    /// `is:ready`.
    Ready,
    /// `is:blocked`.
    Blocked,
    /// `due:<DATE`.
    DueBefore(String),
    /// `due:>DATE`.
    DueAfter(String),
    /// Anything else: a full-text term (a quoted phrase is one term).
    Text(String),
}

/// What a token's value may be, for error messages.
const VALID_KEYS: &str = "state:S, p:P (priority:P), label:L, @WHO, assignee:WHO, project:P, \
is:ready, is:blocked, due:<DATE, due:>DATE";

/// Split on whitespace outside double quotes; quotes group and are removed
/// (`label:"needs review"`, `"empty password"`).
fn words(input: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut any = false;
    for ch in input.chars() {
        match ch {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                }
                any = false;
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

fn bad_token(word: &str) -> Error {
    Error::usage("invalid-filter")
        .with("Token", word)
        .with("Valid", VALID_KEYS)
}

/// `7d` / `2w` are shorthand for `+7d` / `+2w`.
fn due_input(v: &str) -> String {
    let unit = v.ends_with(['d', 'w']);
    let digits = v.len() > 1 && v[..v.len() - 1].bytes().all(|b| b.is_ascii_digit());
    if unit && digits {
        format!("+{v}")
    } else {
        v.to_string()
    }
}

/// Parse a filter string. Only known keys are tokens; other
/// words, `auth.rs:41` included, are free text.
pub fn tokens(input: &str) -> Result<Vec<Token>> {
    let mut out = Vec::new();
    for w in words(input) {
        if let Some(who) = w.strip_prefix('@')
            && !who.is_empty()
        {
            out.push(Token::Assignee(who.to_string()));
            continue;
        }
        let Some((key, value)) = w.split_once(':') else {
            out.push(Token::Text(w));
            continue;
        };
        let key = key.to_ascii_lowercase();
        let known = matches!(
            key.as_str(),
            "state" | "p" | "priority" | "label" | "assignee" | "project" | "is" | "due"
        );
        if !known {
            out.push(Token::Text(w));
            continue;
        }
        let value = value.trim();
        if value.is_empty() {
            return Err(bad_token(&w));
        }
        out.push(match key.as_str() {
            "state" => Token::State(State::parse(value).ok_or_else(|| {
                Error::validation("invalid-state")
                    .with("Value", value)
                    .with("Valid", State::ALL.map(State::as_str).join(", "))
            })?),
            "p" | "priority" => Token::Priority(if value.eq_ignore_ascii_case("none") {
                None
            } else {
                Some(Priority::parse(value).ok_or_else(|| {
                    Error::validation("invalid-priority")
                        .with("Value", value)
                        .with("Valid", "urgent, high, medium, low, none")
                })?)
            }),
            "label" => Token::Label(value.to_string()),
            "assignee" => Token::Assignee(value.to_string()),
            "project" => Token::Project(value.to_string()),
            "is" => match value.to_ascii_lowercase().as_str() {
                "ready" => Token::Ready,
                "blocked" => Token::Blocked,
                _ => return Err(bad_token(&w)),
            },
            _ => match (value.strip_prefix('<'), value.strip_prefix('>')) {
                (Some(d), _) if !d.is_empty() => Token::DueBefore(due_input(d)),
                (_, Some(d)) if !d.is_empty() => Token::DueAfter(due_input(d)),
                _ => return Err(bad_token(&w)),
            },
        });
    }
    Ok(out)
}

impl ListQuery {
    /// Narrow the query by tokens. List criteria (states, priorities,
    /// labels, free text) add to what is there; a single-valued one
    /// (assignee, project, due bound) replaces it.
    pub fn apply(&mut self, tokens: &[Token]) {
        for t in tokens {
            match t {
                Token::State(s) => model::push_unique(&mut self.states, *s),
                Token::Priority(p) => model::push_unique(&mut self.priorities, *p),
                Token::Label(l) => self.labels.push(l.clone()),
                Token::Assignee(w) => self.assignee = Some(w.clone()),
                Token::Project(p) => {
                    self.project = Some(p.clone());
                    self.no_project = false;
                }
                Token::Ready => self.ready = true,
                Token::Blocked => self.blocked = true,
                Token::DueBefore(d) => self.due_before = Some(d.clone()),
                Token::DueAfter(d) => self.due_after = Some(d.clone()),
                Token::Text(t) => self.search.push(t.clone()),
            }
        }
    }

    /// [`tokens`] then [`ListQuery::apply`].
    pub fn filter(&mut self, input: &str) -> Result<()> {
        self.apply(&tokens(input)?);
        Ok(())
    }
}

/// Files handed to the thread pool per round; doubles each round so a short
/// page stays cheap while a long scan amortizes the overhead.
const FIRST_BATCH: usize = 256;
const MAX_BATCH: usize = 8192;

/// Run a query over the store.
pub fn list(store: &Store, me: &Identity, q: &ListQuery) -> Result<Page> {
    let f = Filter::resolve(store, me, q)?;
    let sort_updated = q.sort == Some(Sort::Updated);
    let issues = store.issues()?;
    let by_id: HashMap<&str, &IssueRef> = issues.iter().map(|r| (r.id.as_str(), r)).collect();
    let scoped: Vec<&IssueRef> = issues
        .iter()
        .filter(|r| f.in_scope(r.project.as_ref().map(|p| p.id.as_str())))
        .collect();

    // One file → a row if it passes every filter. Runs on the thread pool.
    let eval = |r: &IssueRef| -> Result<Option<Row>> {
        let raw = match &f.terms {
            Some(t) => {
                let bytes = std::fs::read(&r.path)?;
                if !t.prefilter(&bytes) {
                    return Ok(None);
                }
                Some(bytes)
            }
            None => None,
        };
        let head = match &raw {
            Some(b) => mbox::read_head(&b[..])?,
            None => store.read_head(r)?,
        };
        if !f.matches_fields(&head) {
            return Ok(None);
        }
        if f.ready || f.blocked {
            let mut blocked = false;
            for b in list_of(&head, "X-Blocked-By") {
                if store
                    .state_by_message_id(&b, &by_id)?
                    .is_some_and(State::is_open)
                {
                    blocked = true;
                    break;
                }
            }
            if !f.passes_blocked(blocked) {
                return Ok(None);
            }
        }
        let matches = match (&f.terms, &raw) {
            (Some(t), Some(bytes)) => match t.matches(&store.parse(&r.path, bytes)?) {
                Some(m) => Some(m),
                None => return Ok(None),
            },
            _ => None,
        };
        let stats = match (&raw, sort_updated) {
            (Some(b), true) => Some(mbox::scan_stats(&b[..])?),
            (None, true) => Some(store.stats(r)?),
            _ => None,
        };
        Ok(Some(Row {
            issue: r.clone(),
            head,
            matches,
            stats,
        }))
    };

    // Stream in path order; without a sort stop once the page (+1, to know
    // whether there is more) is full.
    let want = (q.sort.is_none() && q.limit > 0).then(|| q.offset + q.limit + 1);
    let mut rows: Vec<Row> = Vec::new();
    let mut next = 0;
    let mut batch = FIRST_BATCH;
    while next < scoped.len() && want.is_none_or(|w| rows.len() < w) {
        let end = (next + batch).min(scoped.len());
        let found: Vec<Result<Option<Row>>> =
            scoped[next..end].par_iter().map(|r| eval(r)).collect();
        for row in found {
            rows.extend(row?);
        }
        next = end;
        batch = (batch * 2).min(MAX_BATCH);
    }

    if let Some(sort) = q.sort {
        rows.sort_by_cached_key(|r| {
            let updated = r.stats.and_then(|s| s.updated).map(|d| d.timestamp());
            sort_key(sort, &r.head, updated)
        });
    }

    let more = q.limit > 0 && rows.len() > q.offset + q.limit;
    let take = if q.limit == 0 { usize::MAX } else { q.limit };
    let mut rows: Vec<Row> = rows.into_iter().skip(q.offset).take(take).collect();

    if q.stats {
        rows.par_iter_mut()
            .filter(|row| row.stats.is_none())
            .map(|row| -> Result<()> {
                row.stats = Some(store.stats(&row.issue)?);
                Ok(())
            })
            .collect::<Result<()>>()?;
    }
    Ok(Page {
        rows,
        next_offset: more.then_some(q.offset + q.limit),
    })
}

/// What `sort` orders by: (undated, due, state rank, priority rank,
/// newest), with the parts it ignores held constant. `updated` falls back
/// to the creation date.
fn sort_key(
    sort: Sort,
    h: &impl HeadFields,
    updated: Option<i64>,
) -> (bool, Option<i64>, u8, u8, Reverse<i64>) {
    let created = h.created();
    let state = h.state().sort_rank();
    let prio = Priority::rank(h.priority());
    match sort {
        Sort::State => (false, None, state, prio, Reverse(created)),
        Sort::Created => (false, None, 0, 0, Reverse(created)),
        Sort::Updated => (false, None, 0, 0, Reverse(updated.unwrap_or(created))),
        Sort::Priority => (false, None, 0, prio, Reverse(created)),
        Sort::Due => {
            let due = h.due();
            (due.is_none(), due, state, prio, Reverse(created))
        }
    }
}

/// Issue counts of one project.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProjectCounts {
    pub issues: usize,
    pub open: usize,
}

pub fn project_counts(store: &Store, p: &Project) -> Result<ProjectCounts> {
    let issues = store.project_issues(p)?;
    let mut open = 0;
    for r in &issues {
        if state_of(&store.read_head(r)?).is_open() {
            open += 1;
        }
    }
    Ok(ProjectCounts {
        issues: issues.len(),
        open,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Actor;
    use crate::ops::{self, IssueUpdate, NewIssue};
    use crate::store;

    fn setup() -> (tempfile::TempDir, Store, Identity) {
        let dir = tempfile::tempdir().unwrap();
        let root = store::init(dir.path()).unwrap();
        let me = Identity::fixed(Actor {
            from: "Ada <ada@example.com>".into(),
            sender: None,
        });
        (dir, Store::new(root), me)
    }

    proptest::proptest! {
        #[test]
        fn contains_lower_agrees_with_to_lowercase(hay in "[a-zA-Z0-9 _éÉİK-]{0,12}", needle in "[a-zA-Z0-9 éÉİ]{0,3}") {
            let n = needle.to_lowercase();
            proptest::prop_assert_eq!(contains_lower(&hay, &n), hay.to_lowercase().contains(&n));
        }
    }

    fn mk(s: &Store, me: &Identity, n: NewIssue) -> String {
        ops::create_issue(s, me, &n).unwrap().issue.id
    }

    fn titles(p: &Page) -> Vec<&str> {
        p.rows
            .iter()
            .map(|r| r.head.get("Subject").unwrap())
            .collect()
    }

    fn issue(title: &str) -> NewIssue {
        NewIssue {
            title: title.into(),
            ..NewIssue::default()
        }
    }

    #[test]
    fn closed_hidden_unless_asked_and_search_includes_them() {
        let (_d, s, me) = setup();
        mk(&s, &me, issue("open one"));
        let done = mk(&s, &me, issue("closed one"));
        ops::close_issue(&s, &me, &done, &ops::CloseAs::Done, None).unwrap();
        let q = ListQuery::default();
        assert_eq!(titles(&list(&s, &me, &q).unwrap()), ["open one"]);
        let all = ListQuery {
            all: true,
            sort: Some(Sort::State),
            ..q.clone()
        };
        assert_eq!(
            titles(&list(&s, &me, &all).unwrap()),
            ["open one", "closed one"]
        );
        let search = ListQuery {
            search: vec!["CLOSED".into()],
            ..q.clone()
        };
        let p = list(&s, &me, &search).unwrap();
        assert_eq!(titles(&p), ["closed one"]);
        assert_eq!(p.rows[0].matches.as_ref().unwrap().count, 1);
        let narrowed = ListQuery {
            states: vec![State::Backlog],
            ..search
        };
        assert!(list(&s, &me, &narrowed).unwrap().rows.is_empty());
    }

    #[test]
    fn ready_needs_closed_blockers() {
        let (_d, s, me) = setup();
        let blocker = mk(
            &s,
            &me,
            NewIssue {
                state: Some(State::Todo),
                ..issue("blocker")
            },
        );
        mk(
            &s,
            &me,
            NewIssue {
                state: Some(State::Todo),
                blocked_by: vec![blocker.clone()],
                ..issue("blocked")
            },
        );
        let q = ListQuery {
            ready: true,
            ..ListQuery::default()
        };
        assert_eq!(titles(&list(&s, &me, &q).unwrap()), ["blocker"]);
        ops::close_issue(&s, &me, &blocker, &ops::CloseAs::Done, None).unwrap();
        assert_eq!(titles(&list(&s, &me, &q).unwrap()), ["blocked"]);
    }

    #[test]
    fn filters_and_me() {
        let (_d, s, me) = setup();
        mk(
            &s,
            &me,
            NewIssue {
                assignee: Some("me".into()),
                labels: vec!["bug,ui".into()],
                ..issue("mine")
            },
        );
        mk(&s, &me, issue("other"));
        let q = ListQuery {
            assignee: Some("me".into()),
            ..ListQuery::default()
        };
        assert_eq!(titles(&list(&s, &me, &q).unwrap()), ["mine"]);
        let q = ListQuery {
            labels: vec!["UI".into(), "bug".into()],
            ..ListQuery::default()
        };
        assert_eq!(titles(&list(&s, &me, &q).unwrap()), ["mine"]);
        let q = ListQuery {
            agent: true,
            ..ListQuery::default()
        };
        assert!(list(&s, &me, &q).unwrap().rows.is_empty());
    }

    #[test]
    fn paging_and_sorting() {
        let (_d, s, me) = setup();
        for (t, p) in [
            ("a", Priority::Low),
            ("b", Priority::Urgent),
            ("c", Priority::High),
        ] {
            let id = mk(&s, &me, issue(t));
            let u = IssueUpdate {
                priority: Some(Some(p)),
                ..IssueUpdate::default()
            };
            ops::update_issue(&s, &me, &id, &u).unwrap();
        }
        let q = ListQuery {
            sort: Some(Sort::Priority),
            limit: 2,
            stats: true,
            ..ListQuery::default()
        };
        let p = list(&s, &me, &q).unwrap();
        assert_eq!(titles(&p), ["b", "c"]);
        assert_eq!(p.next_offset, Some(2));
        assert_eq!(p.rows[0].stats.unwrap().comments, 0);
        let p = list(&s, &me, &ListQuery { offset: 2, ..q }).unwrap();
        assert_eq!(titles(&p), ["a"]);
        assert_eq!(p.next_offset, None);
    }

    #[test]
    fn project_scope_and_counts() {
        let (_d, s, me) = setup();
        let p = ops::create_project(&s, "Auth").unwrap();
        mk(
            &s,
            &me,
            NewIssue {
                project: Some(p.id.clone()),
                ..issue("in project")
            },
        );
        mk(&s, &me, issue("at root"));
        let q = ListQuery {
            project: Some(p.id.clone()),
            ..ListQuery::default()
        };
        assert_eq!(titles(&list(&s, &me, &q).unwrap()), ["in project"]);
        let q = ListQuery {
            no_project: true,
            ..ListQuery::default()
        };
        assert_eq!(titles(&list(&s, &me, &q).unwrap()), ["at root"]);
        assert_eq!(
            project_counts(&s, &p).unwrap(),
            ProjectCounts { issues: 1, open: 1 }
        );
    }

    #[test]
    fn token_parsing() {
        use Token::*;
        assert_eq!(
            tokens(
                r#"state:todo p:hi label:"needs review" @me is:ready due:<7d auth.rs:41 "empty pw""#
            )
            .unwrap(),
            [
                State(crate::model::State::Todo),
                Priority(Some(crate::model::Priority::High)),
                Label("needs review".into()),
                Assignee("me".into()),
                Ready,
                DueBefore("+7d".into()),
                Text("auth.rs:41".into()),
                Text("empty pw".into()),
            ]
        );
        assert_eq!(
            tokens("P:none priority:Urgent is:Blocked due:>2026-12-01 project:auth assignee:bob")
                .unwrap(),
            [
                Priority(None),
                Priority(Some(crate::model::Priority::Urgent)),
                Blocked,
                DueAfter("2026-12-01".into()),
                Project("auth".into()),
                Assignee("bob".into()),
            ]
        );
        assert!(tokens("").unwrap().is_empty());
        assert_eq!(tokens("state:bogus").unwrap_err().kind, "invalid-state");
        assert_eq!(tokens("p:whenever").unwrap_err().kind, "invalid-priority");
        for bad in ["is:fun", "label:", "due:7d", "due:<"] {
            assert_eq!(tokens(bad).unwrap_err().kind, "invalid-filter", "{bad}");
        }
    }

    #[test]
    fn token_filters() {
        let (_d, s, me) = setup();
        let p = ops::create_project(&s, "Auth work").unwrap();
        let blocker = mk(
            &s,
            &me,
            NewIssue {
                state: Some(State::Todo),
                priority: Some(Priority::High),
                due: Some("+3d".into()),
                project: Some(p.id.clone()),
                ..issue("blocker login")
            },
        );
        mk(
            &s,
            &me,
            NewIssue {
                state: Some(State::Todo),
                blocked_by: vec![blocker],
                due: Some("+30d".into()),
                assignee: Some("me".into()),
                ..issue("blocked login")
            },
        );
        mk(&s, &me, issue("plain"));
        let q = |f: &str| {
            let mut q = ListQuery::default();
            q.filter(f).unwrap();
            titles(&list(&s, &me, &q).unwrap())
                .into_iter()
                .map(str::to_string)
                .collect::<Vec<_>>()
        };
        assert_eq!(q("is:blocked"), ["blocked login"]);
        assert_eq!(q("is:ready"), ["blocker login"]);
        assert_eq!(q("p:high"), ["blocker login"]);
        assert_eq!(q("p:none"), ["blocked login", "plain"]);
        // Path order: root files, then the project folder.
        assert_eq!(q("p:none p:h"), ["blocked login", "plain", "blocker login"]);
        assert_eq!(q("due:<7d"), ["blocker login"]);
        assert_eq!(q("due:>7d"), ["blocked login"]);
        assert_eq!(q("@me"), ["blocked login"]);
        assert_eq!(q("project:auth"), ["blocker login"]);
        assert_eq!(q(&format!("project:{}", p.id)), ["blocker login"]);
        assert_eq!(q("state:backlog"), ["plain"]);
        assert_eq!(q("LOGIN state:todo"), ["blocked login", "blocker login"]);
        let mut bad = ListQuery::default();
        bad.filter("project:nope").unwrap();
        assert_eq!(list(&s, &me, &bad).unwrap_err().code, 2);
    }
}
