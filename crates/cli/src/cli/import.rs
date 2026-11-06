//! `lastissue import linear`. The one place with interactive prompts:
//! on a TTY every missing value is asked for, otherwise it fails fast
//! naming the flag. Prompts only fill the same values as the flags.

use super::common::Ctx;
use super::{ImportCmd, LinearArgs};
use crate::error::{Error, Result};
use crate::format;
use crate::linear::Client;
use crate::linear::import::{Destination, Existing, Import, Known, Tally, find_source};
use crate::store;
use std::io::IsTerminal;

pub fn run(ctx: &mut Ctx, cmd: ImportCmd) -> Result<()> {
    match cmd {
        ImportCmd::Linear(a) => linear(ctx, *a),
    }
}

// ---------------------------------------------------------------- prompts

fn interactive() -> bool {
    std::io::stdin().is_terminal() && std::io::stderr().is_terminal()
}

fn missing(flag: &str) -> Error {
    Error::usage("missing-value").with("Flag", flag).with(
        "Hint",
        "not a terminal, so nothing can be prompted for; pass the flag",
    )
}

fn prompt_err(e: inquire::InquireError) -> Error {
    match e {
        inquire::InquireError::OperationCanceled | inquire::InquireError::OperationInterrupted => {
            Error::usage("aborted")
        }
        e => Error::usage("prompt-failed").with("Message", e.to_string()),
    }
}

/// Pick one of `options` (label, value) and return its value.
fn select<V>(msg: &str, options: Vec<(String, V)>) -> Result<V> {
    if options.is_empty() {
        return Err(Error::not_found("nothing-to-choose").with("Prompt", msg));
    }
    let (labels, mut values): (Vec<_>, Vec<_>) = options.into_iter().unzip();
    let i = inquire::Select::new(msg, labels)
        .with_page_size(15)
        .raw_prompt()
        .map_err(prompt_err)?
        .index;
    Ok(values.swap_remove(i))
}

// ---------------------------------------------------------------- flow

fn tally_fields(t: Tally, prefix: &str) -> Vec<(String, String)> {
    vec![
        (format!("{prefix}Imported"), t.imported.to_string()),
        (format!("{prefix}Updated"), t.updated.to_string()),
        (format!("{prefix}Skipped"), t.skipped.to_string()),
    ]
}

fn linear(ctx: &mut Ctx, a: LinearArgs) -> Result<()> {
    let tty = interactive();

    // API key
    let key = match a
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
    {
        Some(k) => k.to_string(),
        None if tty => inquire::Password::new("Linear API key:")
            .without_confirmation()
            .with_display_mode(inquire::PasswordDisplayMode::Masked)
            .prompt()
            .map_err(prompt_err)?,
        None => return Err(missing("--api-key or LINEAR_API_KEY")),
    };
    let client = Client::new(&key)
        .on_rate_limit(|wait| eprintln!("Rate limited by Linear; retrying in {wait}s"));
    let viewer = client.viewer()?;
    eprintln!(
        "Authenticated as {} @ {}",
        if viewer.name.is_empty() {
            &viewer.email
        } else {
            &viewer.name
        },
        viewer.org
    );

    // Source
    let src = if let Some(q) = &a.project {
        find_source(&client.projects()?, q, "--project")?
    } else if let Some(q) = &a.team {
        find_source(&client.teams()?, q, "--team")?
    } else if tty {
        let by_project = select(
            "Import by:",
            vec![("Project".into(), true), ("Team".into(), false)],
        )?;
        let mut list = if by_project {
            client.projects()?
        } else {
            client.teams()?
        };
        list.retain(|s| a.all || !s.closed);
        list.sort_by_key(|s| s.name.to_lowercase());
        select(
            if by_project { "Project:" } else { "Team:" },
            list.into_iter().map(|s| (s.label(), s)).collect(),
        )?
    } else {
        return Err(missing("--project <name|slug|uuid> or --team <key>"));
    };

    // Destination
    let new_slug = store::slugify(&src.name, 2, "linear");
    let into = if let Some(p) = &a.into {
        Destination::Existing(ctx.store.resolve_project(p)?)
    } else if a.no_project {
        Destination::None
    } else if tty {
        let projects = ctx.store.projects()?;
        // None = pick an existing project next.
        let mut opts = vec![(
            format!("New \"{new_slug}\""),
            Some(Destination::New(new_slug)),
        )];
        if !projects.is_empty() {
            opts.push(("Existing…".into(), None));
        }
        opts.push(("No project".into(), Some(Destination::None)));
        match select("Import into:", opts)? {
            Some(d) => d,
            None => Destination::Existing(select(
                "Project:",
                projects.into_iter().map(|p| (p.dir_name(), p)).collect(),
            )?),
        }
    } else {
        Destination::New(new_slug)
    };

    // What's already here: Message-ID and X-Linear-Id → file.
    let index = Known::scan(&ctx.store)?;

    // Pre-pass (only when a prompt needs it): count + already-imported matches.
    let mut count = None;
    let mut existing = a.existing;
    if tty && (!a.yes || existing.is_none()) {
        let all = client.issue_ids(&src)?;
        let matches = index.already_imported(&all);
        count = Some(all.len());
        if existing.is_none() && matches > 0 {
            existing = Some(select(
                &format!("{matches} already imported:"),
                vec![
                    ("Skip".into(), Existing::Skip),
                    ("Update".into(), Existing::Update),
                ],
            )?);
        }
    }
    let existing = existing.unwrap_or_default();

    if !a.yes {
        if !tty {
            return Err(missing("--yes"));
        }
        let what = match count {
            Some(n) => format!("{n} issues"),
            None => "issues".into(),
        };
        let dest = match &into {
            Destination::New(s) => format!("new project \"{s}\""),
            Destination::Existing(p) => format!("project {}", p.dir_name()),
            Destination::None => "the root (no project)".into(),
        };
        let ok = inquire::Confirm::new(&format!("Import {what} from {} into {dest}?", src.label()))
            .with_default(false)
            .prompt()
            .map_err(prompt_err)?;
        if !ok {
            return Err(Error::usage("aborted"));
        }
    }

    // Stream: fetch a page, write a page.
    let from = ctx.me.actor()?.from.clone();
    ctx.warn_identity();
    let mut run = Import::new(index, into, existing, from);
    let mut after: Option<String> = None;
    let mut page = 0;
    loop {
        let res = client
            .issue_page(&src, after.as_deref())
            .and_then(|(nodes, next)| {
                for n in &nodes {
                    run.node(&ctx.store, n, &src)?;
                }
                Ok((nodes.len(), next))
            });
        let (n, next) = match res {
            Ok(x) => x,
            Err(mut e) => {
                e.fields.extend(tally_fields(run.tally(), ""));
                e.fields.retain(|(k, _)| k != "Hint");
                return Err(e.with(
                    "Hint",
                    "what was written is kept; re-run with --existing skip to continue",
                ));
            }
        };
        page += 1;
        let t = run.tally();
        eprintln!(
            "Page {page}: {n} issues ({} imported, {} updated, {} skipped)",
            t.imported, t.updated, t.skipped
        );
        after = next;
        if after.is_none() {
            break;
        }
    }

    // Summary
    let mut fields = vec![("X-Root".into(), ctx.store.root.display().to_string())];
    fields.push(("X-Source".into(), format!("linear {}", src.label())));
    if let Some(p) = run.project() {
        fields.push(("List-Id".into(), p.list_id()));
    }
    fields.extend(tally_fields(run.tally(), "X-"));
    let mut out = format::block(&fields);
    for (r, head) in run.touched() {
        out.push_str(&format::brief(r, head, ctx.color));
    }
    ctx.print(&out);
    Ok(())
}
