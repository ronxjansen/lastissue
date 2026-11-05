mod cli;
mod format;

use clap::Parser;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use cli::{Cli, Cmd, RepoCmd, common::Ctx};
use error::{Error, Result};
use lastissue::{error, linear, model, registry, store};
use std::io::Write;
use std::path::{Path, PathBuf};

fn main() {
    let code = match Cli::try_parse() {
        Ok(cli) => match run(cli) {
            Ok(out) => {
                let mut stdout = std::io::stdout().lock();
                let _ = stdout.write_all(out.as_bytes());
                0
            }
            Err(e) => {
                eprint!("{e}");
                e.code
            }
        },
        Err(e) => match e.kind() {
            ErrorKind::DisplayHelp
            | ErrorKind::DisplayVersion
            | ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand => {
                let _ = e.print();
                if e.use_stderr() { error::EXIT_USAGE } else { 0 }
            }
            _ => {
                let err = clap_error(&e);
                eprint!("{err}");
                err.code
            }
        },
    };
    std::process::exit(code);
}

/// Explicit root (§2.1): `--root` | `--repo` (clap rejects both), then
/// `LASTISSUE_DIR`. None means walk up, then the global root.
fn explicit_root(root: Option<&Path>, repo: Option<&str>) -> Result<Option<PathBuf>> {
    if let Some(r) = root {
        return Ok(Some(r.to_path_buf()));
    }
    if let Some(label) = repo {
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
    let Cli {
        root,
        repo,
        from,
        sender,
        cmd,
    } = cli;
    let explicit = || explicit_root(root.as_deref(), repo.as_deref());
    match cmd {
        Cmd::Init {
            global,
            no_register,
        } => {
            let dir = if global {
                store::global_root()?
            } else {
                match explicit()? {
                    Some(r) => r,
                    None => store::default_local_root(&std::env::current_dir()?),
                }
            };
            let root = store::init(&dir)?;
            let mut fields = vec![("X-Root".to_string(), root.display().to_string())];
            if !global && !no_register {
                let repo = registry::add(&root)
                    .map_err(|e| e.with("Hint", "lastissue init --no-register"))?;
                fields.push(("X-Repo".into(), repo.label));
            }
            Ok(format::block(&fields))
        }
        Cmd::Repo(c) => Ok(match c {
            RepoCmd::List => repo_blocks(&registry::load()?),
            RepoCmd::Add { path } => {
                let p = match path {
                    Some(p) => p,
                    None => std::env::current_dir()?,
                };
                repo_blocks([&registry::add(&p)?])
            }
            RepoCmd::Remove { repo } => repo_blocks([&registry::remove(&repo)?]),
        }),
        Cmd::Project(c) => in_store(explicit()?, from, sender, |ctx| cli::project::run(ctx, c)),
        Cmd::Issue(c) => in_store(explicit()?, from, sender, |ctx| cli::issue::run(ctx, c)),
        Cmd::Comment(c) => in_store(explicit()?, from, sender, |ctx| cli::comment::run(ctx, c)),
        Cmd::Import(c) => in_store(explicit()?, from, sender, |ctx| cli::import::run(ctx, c)),
    }
}

/// Run a command against the discovered store and return its output.
fn in_store(
    root: Option<PathBuf>,
    from: Option<String>,
    sender: Option<String>,
    f: impl FnOnce(&mut Ctx) -> Result<()>,
) -> Result<String> {
    let mut ctx = Ctx::new(store::discover(root.as_deref())?, from, sender);
    let res = f(&mut ctx);
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
        // `--state <S>` → state; a positional (`[-]`) → value.
        let name = arg
            .trim_start_matches('-')
            .split([' ', '=', '<', '['])
            .next()
            .filter(|n| !n.is_empty())
            .unwrap_or("value");
        return Error::validation(&format!("invalid-{name}"))
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
