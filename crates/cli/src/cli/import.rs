//! `lastissue import linear`. The one place with interactive prompts (§6.2):
//! on a TTY every missing value is asked for, otherwise it fails fast
//! naming the flag. Prompts only fill the same values as the flags.

use super::common::Ctx;
use super::issue::brief;
use super::{ImportCmd, LinearArgs};
use crate::error::{Error, Result};
use crate::format;
use crate::linear::import::{Destination, Existing, Import, Index, Tally, find_source};
use crate::linear::{Client, Source};
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

fn select(msg: &str, options: Vec<String>) -> Result<usize> {
    if options.is_empty() {
        return Err(Error::not_found("nothing-to-choose").with("Prompt", msg));
    }
    Ok(inquire::Select::new(msg, options)
        .with_page_size(15)
        .raw_prompt()
        .map_err(prompt_err)?
        .index)
}

// ---------------------------------------------------------------- flow

fn tally_fields(t: Tally) -> Vec<(String, String)> {
    vec![
        ("X-Imported".into(), t.imported.to_string()),
        ("X-Updated".into(), t.updated.to_string()),
        ("X-Skipped".into(), t.skipped.to_string()),
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
        let by = select("Import by:", vec!["Project".into(), "Team".into()])?;
        let mut list = if by == 0 {
            client.projects()?
        } else {
            client.teams()?
        };
        list.retain(|s| a.all || !s.closed);
        list.sort_by_key(|s| s.name.to_lowercase());
        let i = select(
            if by == 0 { "Project:" } else { "Team:" },
            list.iter().map(Source::label).collect(),
        )?;
        list.swap_remove(i)
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
        let mut opts = vec![format!("New \"{new_slug}\"")];
        if !projects.is_empty() {
            opts.push("Existing…".into());
        }
        opts.push("No project".into());
        match select("Import into:", opts.clone())? {
            0 => Destination::New(new_slug),
            i if i == opts.len() - 1 => Destination::None,
            _ => {
                let i = select("Project:", projects.iter().map(|p| p.dir_name()).collect())?;
                Destination::Existing(projects[i].clone())
            }
        }
    } else {
        Destination::New(new_slug)
    };

    // What's already here: Message-ID and X-Linear-Id → file.
    let index = Index::scan(&ctx.store)?;

    // Pre-pass (only when a prompt needs it): count + already-imported matches.
    let mut count = None;
    let mut existing = a.existing;
    if tty && (!a.yes || existing.is_none()) {
        let all = client.issue_ids(&src)?;
        let matches = index.already_imported(&all);
        count = Some(all.len());
        if existing.is_none() && matches > 0 {
            let i = select(
                &format!("{matches} already imported:"),
                vec!["Skip".into(), "Update".into()],
            )?;
            existing = Some(if i == 0 {
                super::Existing::Skip
            } else {
                super::Existing::Update
            });
        }
    }
    let existing = existing.map_or(Existing::Skip, Into::into);

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
    let from = ctx.actor()?.from.clone();
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
                e.fields.extend(
                    tally_fields(run.tally())
                        .into_iter()
                        .map(|(k, v)| (k.trim_start_matches("X-").to_string(), v)),
                );
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
        match next {
            Some(c) => after = Some(c),
            None => break,
        }
    }

    // Summary
    let mut fields = vec![("X-Root".into(), ctx.store.root.display().to_string())];
    fields.push(("X-Source".into(), format!("linear {}", src.label())));
    if let Some(p) = run.project() {
        fields.push(("List-Id".into(), p.list_id()));
    }
    fields.extend(tally_fields(run.tally()));
    let mut out = format::block(&fields);
    for (r, head) in run.touched() {
        out.push_str(&brief(r, head, ctx.color));
    }
    ctx.print(&out);
    Ok(())
}
