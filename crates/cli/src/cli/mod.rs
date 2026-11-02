//! clap definitions. `--help` is the only documentation agents get, so every
//! subcommand carries concrete examples and enums list their values.

pub mod comment;
pub mod common;
pub mod import;
pub mod issue;
pub mod project;

use crate::model::{Priority, State};
use clap::{Args, Parser, Subcommand, ValueEnum};
use lastissue::linear::import as core_import;
use lastissue::query::Sort;
use std::path::PathBuf;

/// clap mirror of [`State`] (core stays clap-free). Variant order and aliases
/// define `--help` and the `Valid:` list on errors, so keep them in sync.
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum StateArg {
    Backlog,
    Todo,
    #[value(
        alias = "in progress",
        alias = "in_progress",
        alias = "inprogress",
        alias = "doing",
        alias = "wip"
    )]
    InProgress,
    #[value(
        alias = "in review",
        alias = "in_review",
        alias = "inreview",
        alias = "review"
    )]
    InReview,
    #[value(alias = "complete", alias = "completed", alias = "closed")]
    Done,
    #[value(alias = "canceled")]
    Cancelled,
    #[value(alias = "duplicated", alias = "dup")]
    Duplicate,
}

impl From<StateArg> for State {
    fn from(s: StateArg) -> State {
        match s {
            StateArg::Backlog => State::Backlog,
            StateArg::Todo => State::Todo,
            StateArg::InProgress => State::InProgress,
            StateArg::InReview => State::InReview,
            StateArg::Done => State::Done,
            StateArg::Cancelled => State::Cancelled,
            StateArg::Duplicate => State::Duplicate,
        }
    }
}

/// clap mirror of [`Priority`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum PriorityArg {
    #[value(alias = "p0", alias = "u", alias = "critical", alias = "crit")]
    Urgent,
    #[value(alias = "p1", alias = "h", alias = "hi")]
    High,
    #[value(alias = "p2", alias = "m", alias = "med", alias = "normal")]
    Medium,
    #[value(alias = "p3", alias = "l", alias = "lo", alias = "minor")]
    Low,
}

impl From<PriorityArg> for Priority {
    fn from(p: PriorityArg) -> Priority {
        match p {
            PriorityArg::Urgent => Priority::Urgent,
            PriorityArg::High => Priority::High,
            PriorityArg::Medium => Priority::Medium,
            PriorityArg::Low => Priority::Low,
        }
    }
}

const ABOUT: &str = "A local, plaintext issue tracker. One mbox file per issue.";
const LONG_ABOUT: &str = "\
A local, plaintext issue tracker. Every issue is one mbox file (opens in `mutt -f`).
Message #1 holds the issue's current state in its headers; comments and events follow.

Grammar is strictly noun-verb: lastissue <init|repo|project|issue|comment|import> <verb>.
Output is always mbox / RFC 5322 header blocks. Errors are header blocks on stderr.

Root: --root or --repo <label> (a registered repo, see `lastissue repo list`),
then $LASTISSUE_DIR, then the nearest dir with a .lastissue marker
(walking up; at each level ./issues/.lastissue wins over ./.lastissue),
then the global root (`lastissue init --global`).
Identity: --from, then $LASTISSUE_FROM, then git user.name/user.email, then $USER@host.
Agents: also pass --sender \"Agent Name <agent@host>\" (or $LASTISSUE_SENDER).

Exit codes: 0 ok, 1 usage, 2 not found, 3 ambiguous ID prefix, 4 validation.

Examples:
  lastissue init
  lastissue issue create --title \"Login crashes on empty password\" --label bug --priority high
  lastissue issue list --ready --brief
  lastissue issue update 7hq2 --state in-progress --assign me
  lastissue comment create 7hq2 --body \"Reproduced on main.\"
  lastissue issue close 7hq2 --comment \"Fixed in a1b2c3d.\"";

#[derive(Parser, Debug)]
#[command(name = "lastissue", version, about = ABOUT, long_about = LONG_ABOUT)]
pub struct Cli {
    /// Issues root directory (overrides discovery) [env: LASTISSUE_DIR]
    #[arg(
        long,
        global = true,
        value_name = "PATH",
        help_heading = "Global options"
    )]
    pub root: Option<PathBuf>,

    /// Use a registered repo by label (prefix ok; see `lastissue repo list`)
    #[arg(
        long,
        global = true,
        value_name = "LABEL",
        help_heading = "Global options",
        conflicts_with = "root"
    )]
    pub repo: Option<String>,

    /// Author address, "Name <email>" or "email"
    #[arg(
        long,
        global = true,
        env = "LASTISSUE_FROM",
        value_name = "ADDRESS",
        help_heading = "Global options",
        hide_env_values = true
    )]
    pub from: Option<String>,

    /// Acting agent address; written as the Sender: header
    #[arg(
        long,
        global = true,
        env = "LASTISSUE_SENDER",
        value_name = "ADDRESS",
        help_heading = "Global options",
        hide_env_values = true
    )]
    pub sender: Option<String>,

    #[command(subcommand)]
    pub cmd: Cmd,
}

#[derive(Subcommand, Debug)]
pub enum Cmd {
    /// Create an issues root (a .lastissue marker)
    #[command(long_about = "\
Create an issues root: a .lastissue marker in ./issues if that directory exists,
otherwise in the current directory (or exactly in --root), or in the platform
data directory with --global. Idempotent.

A local root is recorded in the repo registry (see `lastissue repo`) unless
--no-register is given. The global root is always listed and never recorded.

Examples:
  lastissue init
  lastissue init --global
  lastissue --root ~/notes/issues init --no-register")]
    Init {
        /// Create the global root instead of a local one
        #[arg(long)]
        global: bool,
        /// Don't record the root in the repo registry
        #[arg(long)]
        no_register: bool,
    },
    /// Manage the registry of known issue roots
    #[command(subcommand)]
    Repo(RepoCmd),
    /// Manage projects (flat folders named <slug>-<id>)
    #[command(subcommand)]
    Project(ProjectCmd),
    /// Manage issues
    #[command(subcommand)]
    Issue(IssueCmd),
    /// Manage comments on an issue
    #[command(subcommand)]
    Comment(CommentCmd),
    /// Import issues from another tracker
    #[command(subcommand)]
    Import(ImportCmd),
}

// ---------------------------------------------------------------- repo

#[derive(Subcommand, Debug)]
pub enum RepoCmd {
    /// List registered repos
    #[command(long_about = "\
List registered repos as header blocks (X-Repo, X-Root, X-Repo-Status: ok | missing).
The label is the folder containing issues/ (or the root's own folder name). The global
root is always listed last, as `global`. Missing roots are shown, never pruned.

The registry is a plain file of root paths: $LASTISSUE_REPOS, else <config dir>/repos.

Examples:
  lastissue repo list
  lastissue --repo web issue list --brief")]
    List,
    /// Register an existing root
    #[command(long_about = "\
Register an existing root (e.g. in a fresh clone; `init` registers automatically).
PATH is a root or a directory containing issues/; it defaults to the current
directory. Idempotent.

Examples:
  lastissue repo add
  lastissue repo add ~/src/web")]
    Add {
        /// Root, or a directory containing issues/ [default: current directory]
        path: Option<PathBuf>,
    },
    /// Unregister a repo (files are not touched)
    #[command(long_about = "\
Unregister a repo by label (prefix ok) or path. Nothing on disk besides the
registry file is touched. The global root cannot be removed.

Examples:
  lastissue repo remove web
  lastissue repo remove ~/src/old/issues")]
    Remove {
        /// Label (prefix ok) or root path
        #[arg(value_name = "LABEL|PATH")]
        repo: String,
    },
}

// ---------------------------------------------------------------- import

#[derive(Subcommand, Debug)]
pub enum ImportCmd {
    /// Import one Linear project or team
    #[command(long_about = "\
Import one Linear project or team. On a terminal, missing values are prompted for;
every prompt has a flag, and without a terminal a missing value fails fast naming it.
The API key (a Linear personal API key) is never stored.

Issues keep their original author, dates and comments (Sender: lastissue-import).
Message-IDs are derived from the Linear issues, so re-runs find what was already
imported (X-Linear-Id): --existing skip leaves those alone, --existing update lets
Linear win on the fields it owns, appends/updates Linear comments in place and
appends one update event. Nothing is ever deleted locally.

Examples:
  LINEAR_API_KEY=lin_api_… lastissue import linear --project \"Billing Revamp\" --yes
  lastissue import linear --team ENG --into 45ae --existing update --yes
  lastissue import linear      # interactive")]
    Linear(Box<LinearArgs>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum Existing {
    /// Leave already-imported issues untouched
    Skip,
    /// Linear wins on the fields it owns; local-only headers and comments stay
    Update,
}

impl From<Existing> for core_import::Existing {
    fn from(e: Existing) -> core_import::Existing {
        match e {
            Existing::Skip => core_import::Existing::Skip,
            Existing::Update => core_import::Existing::Update,
        }
    }
}

#[derive(Args, Debug)]
pub struct LinearArgs {
    /// Linear personal API key
    #[arg(
        long,
        env = "LINEAR_API_KEY",
        value_name = "KEY",
        hide_env_values = true
    )]
    pub api_key: Option<String>,
    /// Linear project to import: name, slug or UUID
    #[arg(long, value_name = "NAME|SLUG|UUID", conflicts_with = "team")]
    pub project: Option<String>,
    /// Linear team to import, by key (e.g. ENG)
    #[arg(long, value_name = "KEY")]
    pub team: Option<String>,
    /// Also list completed / cancelled Linear projects in the picker
    #[arg(long)]
    pub all: bool,
    /// Import into this existing local project (id prefix)
    #[arg(long, value_name = "P", conflicts_with = "no_project")]
    pub into: Option<String>,
    /// Import to the root, outside any project
    #[arg(long)]
    pub no_project: bool,
    /// What to do with issues imported before [default: skip]
    #[arg(long, value_enum, ignore_case = true, value_name = "MODE")]
    pub existing: Option<Existing>,
    /// Don't ask for confirmation
    #[arg(long, short = 'y')]
    pub yes: bool,
}

// ---------------------------------------------------------------- project

#[derive(Subcommand, Debug)]
pub enum ProjectCmd {
    /// Create a project folder
    #[command(long_about = "\
Create a project: a folder named <slug>-<id>. The slug is the first 2 words of NAME
in kebab-case; the id becomes the project's List-Id.

Examples:
  lastissue project create \"New feature\"     # → new-feature-45aef6/")]
    Create {
        /// Project name (first 2 words become the slug)
        name: String,
    },
    /// List projects with issue counts
    #[command(long_about = "\
List projects as header blocks (List-Id, X-Short-Id, X-Path, X-Issue-Count, X-Open-Count).

Examples:
  lastissue project list")]
    List,
    /// Rename a project (changes the slug, keeps the id)
    #[command(long_about = "\
Rename a project: the folder slug changes, the id suffix stays. List-Id headers of the
project's issues are updated.

Examples:
  lastissue project rename 45ae \"Auth rework\"")]
    Rename {
        /// Project id prefix (or folder name)
        project: String,
        /// New name (first 2 words become the slug)
        name: String,
    },
    /// Delete a project folder
    #[command(long_about = "\
Delete a project folder. Refuses when it still contains issues, unless --force
(which deletes those issues too).

Examples:
  lastissue project delete 45ae
  lastissue project delete 45ae --force")]
    Delete {
        /// Project id prefix (or folder name)
        project: String,
        /// Delete even if the project contains issues
        #[arg(long)]
        force: bool,
    },
}

// ---------------------------------------------------------------- shared args

#[derive(Args, Debug, Default, Clone)]
pub struct BodyArgs {
    /// Body text (markdown)
    #[arg(long, conflicts_with_all = ["body_file", "stdin"])]
    pub body: Option<String>,
    /// Read the body from a file ("-" = stdin)
    #[arg(long, value_name = "PATH", conflicts_with = "stdin")]
    pub body_file: Option<PathBuf>,
    /// Pass "-" to read the body from stdin
    #[arg(value_name = "-", value_parser = parse_dash)]
    pub stdin: Option<bool>,
}

fn parse_dash(s: &str) -> Result<bool, String> {
    if s == "-" {
        Ok(true)
    } else {
        Err(format!(
            "unexpected argument '{s}'; only \"-\" (read body from stdin) is accepted here"
        ))
    }
}

// ---------------------------------------------------------------- issue

#[derive(Subcommand, Debug)]
pub enum IssueCmd {
    /// Create an issue
    #[command(long_about = "\
Create an issue. The filename is <slug>-<id>.mbox, the slug being the first 4 words of
the title (frozen forever). The body comes from --body, --body-file, \"-\" or piped stdin.
Dates accept YYYY-MM-DD, today, tomorrow, +Nd, +Nw or a weekday name.

Examples:
  lastissue issue create --title \"Login crashes on empty password\" --label bug,auth --priority high
  git diff | lastissue issue create --title \"Review this diff\" --project 45ae --assign me -
  lastissue issue create --title \"Write docs\" --state todo --due friday --blocked-by 7hq2")]
    Create(Box<CreateArgs>),
    /// List issues (open ones by default)
    #[command(long_about = "\
List issues as header blocks (message #1 headers + computed X-Short-Id, X-Path,
X-Comment-Count, X-Updated), or one line each with --brief. Closed issues are hidden
unless --all or an explicit --state (--search includes them unless --state is given).
WHO accepts \"me\".

Order is path order (project folder, then filename); --sort state gives in-progress,
in-review, todo, backlog, closed, then priority, then newest. When more results may
exist, a final record says how to get the next page: X-Next: --offset N.

--search matches the subject and the bodies of the issue and its comments (not
headers or events). Repeated terms must all match. Each hit gets X-Match lines
(location subject, body or /n for comment n, plus a snippet) and X-Match-Count.

--filter takes the token grammar the GUI search box uses: state:S, p:P (or
priority:P, none for no priority), label:L, @WHO or assignee:WHO, project:P (id
prefix or slug), is:ready, is:blocked, due:<DATE and due:>DATE (DATE as for --due;
7d means +7d). Quotes group words (label:\"needs review\"). Other words are --search
terms. Tokens add to the flags; a token for assignee, project or due replaces it.

Examples:
  lastissue issue list --brief
  lastissue issue list --ready --assignee me
  lastissue issue list --state done --label bug --sort updated --limit 10
  lastissue issue list --search \"empty password\" --search auth
  lastissue issue list --search 'unwrap\\(\\)' --regex --brief --offset 50
  lastissue issue list --filter \"@me p:high is:ready\"
  lastissue issue list --filter \"state:todo label:bug due:<7d login crash\"")]
    List(Box<ListArgs>),
    /// Print an issue's full mbox content
    #[command(long_about = "\
Print the issue file as mbox, with computed headers added (X-Short-Id, X-Path, X-Root,
X-Comment-Count, X-Updated on the issue; X-Comment: <id>/<n> on comments, and
X-Replying-To: ↳ replying to /n on threaded replies). --no-events leaves out event
messages (X-Issue-Event), showing only the issue and its comments.

Examples:
  lastissue issue show 7hq2
  lastissue issue show 7hq2 --no-events
  lastissue issue show 7hq2mz | grep '^X-Issue-State'")]
    Show {
        /// Issue id (any unambiguous prefix)
        id: String,
        /// Leave out event messages (X-Issue-Event)
        #[arg(long)]
        no_events: bool,
    },
    /// Change an issue's fields (appends one event listing every change)
    #[command(long_about = "\
Update an issue. Header changes are rewritten in message #1 and one event message
(X-Issue-Event: update) lists every changed field. A body edit is done in place,
marked with X-Edited and recorded as \"body: edited\". Changes by the same From and
Sender within 15 minutes of an update event that is still the last message merge
into it (original \"from\" kept, fields changed back dropped). The filename never
changes.

Examples:
  lastissue issue update 7hq2 --state in-progress --assign me
  lastissue issue update 7hq2 --add-label regression --remove-label triage --priority urgent
  lastissue issue update 7hq2 --blocked-by 9m2c --header X-Estimate=3")]
    Update(Box<UpdateArgs>),
    /// Close an issue (done, cancelled or duplicate)
    #[command(long_about = "\
Close an issue: done by default, cancelled with --cancel, duplicate with --duplicate-of
(which also sets References: to the original's Message-ID).

Examples:
  lastissue issue close 7hq2
  lastissue issue close 7hq2 --comment \"Fixed in a1b2c3d.\"
  lastissue issue close 7hq2 --duplicate-of 9m2c")]
    Close {
        /// Issue id (any unambiguous prefix)
        id: String,
        /// Close as cancelled instead of done
        #[arg(long, conflicts_with = "duplicate_of")]
        cancel: bool,
        /// Close as duplicate of another issue
        #[arg(long, value_name = "ID")]
        duplicate_of: Option<String>,
        /// Also add a comment
        #[arg(long, value_name = "TEXT")]
        comment: Option<String>,
    },
    /// Reopen an issue (state → todo)
    #[command(long_about = "\
Reopen a closed issue: state becomes todo (and a duplicate's References is cleared).

Examples:
  lastissue issue reopen 7hq2")]
    Reopen {
        /// Issue id (any unambiguous prefix)
        id: String,
    },
    /// Move an issue into a project or out of all projects
    #[command(long_about = "\
Move an issue file into a project folder (--project) or to the root (--no-project).
Updates List-Id and appends a move event.

Examples:
  lastissue issue move 7hq2 --project 45ae
  lastissue issue move 7hq2 --no-project")]
    Move {
        /// Issue id (any unambiguous prefix)
        id: String,
        /// Target project id prefix
        #[arg(
            long,
            value_name = "P",
            required_unless_present = "no_project",
            conflicts_with = "no_project"
        )]
        project: Option<String>,
        /// Move to the root (no project)
        #[arg(long)]
        no_project: bool,
    },
    /// Delete an issue file
    #[command(long_about = "\
Delete an issue: removes its file. History remains only in git, if you use it.

Examples:
  lastissue issue delete 7hq2")]
    Delete {
        /// Issue id (any unambiguous prefix)
        id: String,
    },
}

#[derive(Args, Debug)]
pub struct CreateArgs {
    /// Title (Subject:)
    #[arg(long)]
    pub title: String,
    /// Project id prefix
    #[arg(long, value_name = "P")]
    pub project: Option<String>,
    #[command(flatten)]
    pub body: BodyArgs,
    /// Initial state [default: backlog]
    #[arg(long, value_enum, ignore_case = true, value_name = "S")]
    pub state: Option<StateArg>,
    /// Priority
    #[arg(long, value_enum, ignore_case = true, value_name = "P")]
    pub priority: Option<PriorityArg>,
    /// Label (repeatable, comma-separated ok)
    #[arg(long, value_name = "L")]
    pub label: Vec<String>,
    /// Assignee address or "me"
    #[arg(long, value_name = "WHO")]
    pub assign: Option<String>,
    /// Watcher address or "me" (repeatable)
    #[arg(long, value_name = "WHO")]
    pub watch: Vec<String>,
    /// Due date (Reply-By:)
    #[arg(long, value_name = "DATE")]
    pub due: Option<String>,
    /// Stale date (Expires:), display only
    #[arg(long, value_name = "DATE")]
    pub expires: Option<String>,
    /// Parent issue id
    #[arg(long, value_name = "ID")]
    pub parent: Option<String>,
    /// Blocking issue id (repeatable)
    #[arg(long, value_name = "ID")]
    pub blocked_by: Vec<String>,
    /// Related issue id (repeatable)
    #[arg(long, value_name = "ID")]
    pub relate: Vec<String>,
    /// Extra header K=V: X-* (custom), Comments, Sensitivity (repeatable)
    #[arg(long, value_name = "K=V")]
    pub header: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, ValueEnum)]
pub enum SortKey {
    /// State (in-progress → closed), then priority, then newest
    State,
    Created,
    Updated,
    Priority,
    Due,
}

impl From<SortKey> for Sort {
    fn from(s: SortKey) -> Sort {
        match s {
            SortKey::State => Sort::State,
            SortKey::Created => Sort::Created,
            SortKey::Updated => Sort::Updated,
            SortKey::Priority => Sort::Priority,
            SortKey::Due => Sort::Due,
        }
    }
}

#[derive(Args, Debug)]
pub struct ListArgs {
    /// Only issues in this project (id prefix)
    #[arg(long, value_name = "P", conflicts_with = "no_project")]
    pub project: Option<String>,
    /// Only issues without a project
    #[arg(long)]
    pub no_project: bool,
    /// Only these states (repeatable; shows closed states too)
    #[arg(long, value_enum, ignore_case = true, value_name = "S")]
    pub state: Vec<StateArg>,
    /// Include closed issues
    #[arg(long)]
    pub all: bool,
    /// Has all these labels (repeatable)
    #[arg(long, value_name = "L")]
    pub label: Vec<String>,
    /// Assignee (To:) matches WHO
    #[arg(long, value_name = "WHO")]
    pub assignee: Option<String>,
    /// Author (From:) matches WHO
    #[arg(long, value_name = "WHO")]
    pub author: Option<String>,
    /// Watchers (Cc:) include WHO
    #[arg(long, value_name = "WHO")]
    pub watching: Option<String>,
    /// Only issues written by an agent (have Sender:)
    #[arg(long)]
    pub agent: bool,
    /// Only children of this issue
    #[arg(long, value_name = "ID")]
    pub parent: Option<String>,
    /// Only todo issues whose blockers are all closed
    #[arg(long)]
    pub ready: bool,
    /// Only issues past their Expires: date
    #[arg(long)]
    pub expired: bool,
    /// Case-insensitive text in the subject, body or comments (repeatable, ANDed)
    #[arg(long, value_name = "TEXT")]
    pub search: Vec<String>,
    /// Treat every --search term as a regex (ripgrep syntax)
    #[arg(long, requires = "search")]
    pub regex: bool,
    /// Filter tokens: state:S p:P label:L @WHO assignee:WHO project:P is:ready
    /// is:blocked due:<DATE due:>DATE; other words are --search terms (repeatable)
    #[arg(long, value_name = "TOKENS")]
    pub filter: Vec<String>,
    /// Sort order (scans every issue) [default: path order]
    #[arg(long, value_enum, ignore_case = true)]
    pub sort: Option<SortKey>,
    /// Maximum results; 0 = unlimited
    #[arg(long, value_name = "N", default_value_t = 50)]
    pub limit: usize,
    /// Skip the first N results (see X-Next)
    #[arg(long, value_name = "N", default_value_t = 0)]
    pub offset: usize,
    /// One line per issue: id, state, priority, labels, title
    #[arg(long)]
    pub brief: bool,
}

#[derive(Args, Debug)]
pub struct UpdateArgs {
    /// Issue id (any unambiguous prefix)
    pub id: String,
    /// New title (the filename slug stays frozen)
    #[arg(long)]
    pub title: Option<String>,
    #[command(flatten)]
    pub body: BodyArgs,
    /// New state
    #[arg(long, value_enum, ignore_case = true, value_name = "S")]
    pub state: Option<StateArg>,
    /// New priority
    #[arg(
        long,
        value_enum,
        ignore_case = true,
        value_name = "P",
        conflicts_with = "no_priority"
    )]
    pub priority: Option<PriorityArg>,
    /// Remove the priority
    #[arg(long)]
    pub no_priority: bool,
    /// Add label (repeatable)
    #[arg(long, value_name = "L")]
    pub add_label: Vec<String>,
    /// Remove label (repeatable)
    #[arg(long, value_name = "L")]
    pub remove_label: Vec<String>,
    /// Set the assignee (address or "me")
    #[arg(long, value_name = "WHO", conflicts_with = "unassign")]
    pub assign: Option<String>,
    /// Remove the assignee
    #[arg(long)]
    pub unassign: bool,
    /// Add a watcher (repeatable)
    #[arg(long, value_name = "WHO")]
    pub watch: Vec<String>,
    /// Remove a watcher (repeatable)
    #[arg(long, value_name = "WHO")]
    pub unwatch: Vec<String>,
    /// Set the due date
    #[arg(long, value_name = "DATE", conflicts_with = "no_due")]
    pub due: Option<String>,
    /// Remove the due date
    #[arg(long)]
    pub no_due: bool,
    /// Set the stale date ("" removes it)
    #[arg(long, value_name = "DATE")]
    pub expires: Option<String>,
    /// Set the parent issue
    #[arg(long, value_name = "ID", conflicts_with = "no_parent")]
    pub parent: Option<String>,
    /// Remove the parent
    #[arg(long)]
    pub no_parent: bool,
    /// Add a blocker (repeatable)
    #[arg(long, value_name = "ID")]
    pub blocked_by: Vec<String>,
    /// Remove a blocker (repeatable)
    #[arg(long, value_name = "ID")]
    pub unblock: Vec<String>,
    /// Add a related issue (repeatable)
    #[arg(long, value_name = "ID")]
    pub relate: Vec<String>,
    /// Remove a related issue (repeatable)
    #[arg(long, value_name = "ID")]
    pub unrelate: Vec<String>,
    /// Set header K=V (empty V removes): X-*, Comments, Sensitivity (repeatable)
    #[arg(long, value_name = "K=V")]
    pub header: Vec<String>,
}

// ---------------------------------------------------------------- comment

#[derive(Subcommand, Debug)]
pub enum CommentCmd {
    /// Add a comment to an issue
    #[command(long_about = "\
Append a comment (a reply message). Flat by default; --reply-to threads it under an
earlier comment. The body comes from --body, --body-file, \"-\" or piped stdin; on a
terminal with no body, $EDITOR opens.

Examples:
  lastissue comment create 7hq2 --body \"Reproduced on main.\"
  cargo test 2>&1 | lastissue comment create 7hq2 -
  lastissue comment create 7hq2 --reply-to 7hq2/1 --body \"Agreed.\"")]
    Create {
        /// Issue id (any unambiguous prefix)
        id: String,
        #[command(flatten)]
        body: BodyArgs,
        /// Thread under a comment: <issue>/<n> or a comment Message-ID prefix
        #[arg(long, value_name = "ID/N")]
        reply_to: Option<String>,
    },
    /// List an issue's comments
    #[command(long_about = "\
List an issue's comments (events excluded) as mbox, each with X-Comment: <id>/<n>.

Examples:
  lastissue comment list 7hq2")]
    List {
        /// Issue id (any unambiguous prefix)
        id: String,
    },
    /// Edit a comment in place (marks X-Edited)
    #[command(long_about = "\
Replace a comment's body in place; adds X-Edited (and X-Edited-By when you are not the
author). Address it as <issue>/<n> (n counts comments from 1) or by Message-ID prefix.

Examples:
  lastissue comment update 7hq2/2 --body \"Corrected: it fails on Linux only.\"")]
    Update {
        /// <issue>/<n> or comment Message-ID prefix
        comment: String,
        #[command(flatten)]
        body: BodyArgs,
    },
    /// Delete a comment (appends a comment-deleted event)
    #[command(long_about = "\
Remove a comment message and append a comment-deleted event.

Examples:
  lastissue comment delete 7hq2/2")]
    Delete {
        /// <issue>/<n> or comment Message-ID prefix
        comment: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every clap name and alias must parse to the same value in core.
    #[test]
    fn mirror_enums_match_core_parse() {
        for v in StateArg::value_variants() {
            for name in v.to_possible_value().unwrap().get_name_and_aliases() {
                assert_eq!(State::parse(name), Some(State::from(*v)), "{name}");
            }
        }
        for v in PriorityArg::value_variants() {
            for name in v.to_possible_value().unwrap().get_name_and_aliases() {
                assert_eq!(Priority::parse(name), Some(Priority::from(*v)), "{name}");
            }
        }
    }
}
