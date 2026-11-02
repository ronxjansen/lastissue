mod cli;
mod dates;
mod error;
mod format;
mod model;
mod store;

use clap::Parser;
use clap::error::{ContextKind, ContextValue, ErrorKind};
use cli::{Cli, Cmd, common::Ctx};
use error::{Error, Result};
use std::io::Write;

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

fn run(cli: Cli) -> Result<String> {
    if let Cmd::Init { global } = cli.cmd {
        let dir = if global {
            store::global_root()?
        } else {
            match &cli.root {
                Some(r) => r.clone(),
                None => store::default_local_root(&std::env::current_dir()?),
            }
        };
        let root = store::init(&dir)?;
        return Ok(format::block(&[(
            "X-Root".into(),
            root.display().to_string(),
        )]));
    }
    let root = store::discover(cli.root.as_deref())?;
    let mut ctx = Ctx::new(root, cli.from, cli.sender);
    match cli.cmd {
        Cmd::Init { .. } => unreachable!(),
        Cmd::Project(c) => cli::project::run(&mut ctx, c)?,
        Cmd::Issue(c) => cli::issue::run(&mut ctx, c)?,
        Cmd::Comment(c) => cli::comment::run(&mut ctx, c)?,
    }
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
