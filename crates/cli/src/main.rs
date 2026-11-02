mod cli;
mod format;

use clap::Parser;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use cli::{Cli, Cmd, RepoCmd, common::Ctx};
use error::{Error, Result};
use lastissue::{error, linear, model, registry, store};
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let code = match Cli::try_parse() {
        Ok(cli) => match run(cli) {
            Ok(out) => {
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(out.as_bytes());
                0
            }
            Err(e) => {
                format::print_error(&e);
                e.code
            }
        },
        Err(e) => match e.kind() {
            ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                let _ = e.print();
                if e.kind() == ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand {
                    error::EXIT_USAGE
                } else {
                    0
                }
            }
            _ => {
                let err = clap_error(&e);
                format::print_error(&err);
                err.code
            }
        },
    };
    std::process::exit(code);
}

/// Explicit root (§2.1): `--root` | `--repo` (clap rejects both), then
/// `LASTISSUE_DIR`. None means walk up, then the global root.
fn explicit_root(cli: &Cli) -> Result<Option<PathBuf>> {
    if let Some(r) = &cli.root {
        return Ok(Some(r.clone()));
    }
    if let Some(label) = &cli.repo {
        return Ok(Some(registry::resolve(label)?.root));
    }
    Ok(std::env::var_os("LASTISSUE_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from))
}

fn repo_blocks<'a>(repos: impl IntoIterator<Item = &'a registry::Repo>) -> String {
    repos
        .into_iter()
        .map(|r| format::block(&r.fields()))
        .collect::<Vec<_>>()
        .join("\n")
}

fn run(cli: Cli) -> Result<String> {
    match &cli.cmd {
        Cmd::Init {
            global,
            no_register,
        } => {
            let dir = if *global {
                store::global_root()?
            } else {
                match explicit_root(&cli)? {
                    Some(r) => r,
                    None => store::default_local_root(&std::env::current_dir()?),
                }
            };
            let root = store::init(&dir)?;
            let mut fields = vec![("X-Root".to_string(), root.display().to_string())];
            if !*global && !*no_register {
                let repo = registry::add(&root)
                    .map_err(|e| e.with("Hint", "lastissue init --no-register"))?;
                fields.push(("X-Repo".into(), repo.label));
            }
            return Ok(format::block(&fields));
        }
        Cmd::Repo(c) => {
            return Ok(match c {
                RepoCmd::List => repo_blocks(&registry::load()?),
                RepoCmd::Add { path } => {
                    let p = match path {
                        Some(p) => p.clone(),
                        None => std::env::current_dir()?,
                    };
                    repo_blocks([&registry::add(&p)?])
                }
                RepoCmd::Remove { repo } => repo_blocks([&registry::remove(repo)?]),
            });
        }
        _ => {}
    }
    let root = store::discover(explicit_root(&cli)?.as_deref())?;
    let mut ctx = Ctx::new(root, cli.from, cli.sender);
    let res = match cli.cmd {
        Cmd::Init { .. } | Cmd::Repo(_) => unreachable!(),
        Cmd::Project(c) => cli::project::run(&mut ctx, c),
        Cmd::Issue(c) => cli::issue::run(&mut ctx, c),
        Cmd::Comment(c) => cli::comment::run(&mut ctx, c),
        Cmd::Import(c) => cli::import::run(&mut ctx, c),
    };
    // Core resolves identity lazily and never prints: warn here if the
    // command fell back to $USER@host.
    ctx.warn_identity();
    res?;
    Ok(ctx.take_output())
}

/// Turn a clap error into a header block: invalid enum values are validation
/// errors (exit 4) naming the valid values; everything else is usage (exit 1).
fn clap_error(e: &clap::Error) -> Error {
    let ctx_str = |k| match e.get(k) {
        Some(ContextValue::String(s)) => Some(s.clone()),
        _ => None,
    };
    if e.kind() == ErrorKind::InvalidValue
        && let Some(ContextValue::Strings(valid)) = e.get(ContextKind::ValidValue)
    {
        let arg = ctx_str(ContextKind::InvalidArg).unwrap_or_default();
        let name = arg
            .trim_start_matches('-')
            .split([' ', '=', '<'])
            .next()
            .unwrap_or("value")
            .to_string();
        return Error::validation(&format!(
            "invalid-{}",
            if name.is_empty() { "value" } else { &name }
        ))
        .with(
            "Value",
            ctx_str(ContextKind::InvalidValue).unwrap_or_default(),
        )
        .with("Valid", valid.join(", "));
    }
    let rendered = e.render().to_string();
    let mut err = Error::usage("usage");
    for line in rendered.lines() {
        let l = line.trim();
        if let Some(m) = l.strip_prefix("error: ") {
            err = err.with("Message", m);
        } else if let Some(t) = l.strip_prefix("tip: ") {
            err = err.with("Tip", t);
        } else if let Some(u) = l.strip_prefix("Usage: ") {
            err = err.with("Usage", u);
        }
    }
    err.with("Hint", "add --help to any command for examples")
}
