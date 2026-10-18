// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! `weft-boardchecker`: checks a PCB netlist against a device tree overlay.

use anyhow::{Context, Result, bail};
use clap::Parser;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use weft_boardchecker::check::{self, Input, Severity};
use weft_boardchecker::config::Config;
use weft_boardchecker::host::Host;
use weft_boardchecker::overlay::Overlay;
use weft_boardchecker::parts::Library;
use weft_boardchecker::{board::Board, bom, ipc356, ipc2581};

/// Checks that a board's I2C/SPI devices, addresses, chip selects and GPIOs match an overlay.
#[derive(Debug, Parser)]
#[command(version)]
struct Args {
    /// Board export: IPC-2581 XML, or IPC-D-356A (then also pass --bom).
    #[arg(long)]
    board: PathBuf,
    /// Compiled overlay (.dtbo, built with `dtc -@`).
    #[arg(long)]
    overlay: PathBuf,
    /// CSV bill of materials with designator and part number columns.
    #[arg(long)]
    bom: Option<PathBuf>,
    /// Board configuration file.
    #[arg(long, short)]
    config: Option<PathBuf>,
    /// Host profile name or file (overrides the config). Default: raspberry-pi-40pin.
    #[arg(long)]
    host: Option<String>,
    /// Designator of the host connector (overrides the config).
    #[arg(long)]
    connector: Option<String>,
    /// Extra parts library (repeatable); takes precedence over the built-in one.
    #[arg(long)]
    parts: Vec<PathBuf>,
    /// Also print passed checks and informational notes.
    #[arg(long, short)]
    verbose: bool,
    /// Fail on warnings too.
    #[arg(long)]
    deny_warnings: bool,
}

#[allow(clippy::print_stdout)]
fn main() -> ExitCode {
    match run() {
        Ok(code) => code,
        Err(error) => {
            println!("error: {error:#}");
            ExitCode::from(2)
        }
    }
}

#[allow(clippy::print_stdout)]
fn run() -> Result<ExitCode> {
    let args = Args::parse();
    let config = match &args.config {
        Some(path) => Config::load(path)?,
        None => Config::default(),
    };

    let host = load_host(args.host.as_deref().or(config.host.as_deref()), &config)?;
    let Some(connector) = args.connector.clone().or_else(|| config.connector.clone()) else {
        bail!("no host connector given; pass --connector or set `connector` in the config");
    };

    let mut parts = Library::builtin()?;
    let extra: Vec<PathBuf> = config
        .parts
        .iter()
        .map(|p| config.resolve(p))
        .chain(args.parts.clone())
        .collect();
    for path in extra.iter().rev() {
        parts.add_toml(&read_text(path)?, &path.display().to_string())?;
    }

    let mut board = load_board(&args.board, args.bom.as_deref(), &config)?;
    for (refdes, mpn) in &config.mpn {
        board.set_mpn(refdes, mpn);
    }

    let blob = std::fs::read(&args.overlay)
        .with_context(|| format!("cannot read {}", args.overlay.display()))?;
    let overlay = Overlay::parse(&blob).with_context(|| args.overlay.display().to_string())?;

    let expect: BTreeMap<String, String> = config
        .expect
        .iter()
        .map(|e| (e.refdes.clone(), e.reason.clone()))
        .collect();
    let ignore: BTreeSet<String> = config.ignore.iter().cloned().collect();
    let findings = check::check(Input {
        board: &board,
        overlay: &overlay,
        host: &host,
        parts: &parts,
        connector: &connector,
        expect: &expect,
        ignore: &ignore,
    });

    let (mut errors, mut warnings) = (0usize, 0usize);
    let width = findings.iter().map(|f| f.subject.len()).max().unwrap_or(0);
    for finding in &findings {
        let tag = match finding.severity {
            Severity::Error => {
                errors += 1;
                "error"
            }
            Severity::Warning => {
                warnings += 1;
                "warn"
            }
            Severity::Info if args.verbose => "info",
            Severity::Ok if args.verbose => "ok",
            Severity::Info | Severity::Ok => continue,
        };
        println!("{tag:<6} {:<width$}  {}", finding.subject, finding.message);
    }
    println!(
        "{errors} error(s), {warnings} warning(s); {} component(s), {} overlay fragment(s), host {}",
        board.components.len(),
        overlay.fragments.len(),
        host.name
    );
    Ok(if errors > 0 || (args.deny_warnings && warnings > 0) {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    })
}

fn read_text(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))
}

fn load_host(spec: Option<&str>, config: &Config) -> Result<Host> {
    let spec = spec.unwrap_or("raspberry-pi-40pin");
    if Host::builtin_names().any(|n| n == spec) {
        return Host::builtin(spec);
    }
    let path = config.resolve(Path::new(spec));
    if path.exists() {
        return Host::from_toml(&read_text(&path)?, &path.display().to_string());
    }
    Host::builtin(spec)
}

fn load_board(path: &Path, bom_path: Option<&Path>, config: &Config) -> Result<Board> {
    let text = read_text(path)?;
    let is_xml = text.trim_start().starts_with('<');
    let mut board = if is_xml {
        ipc2581::parse(&text).with_context(|| path.display().to_string())?
    } else {
        ipc356::parse(&text).with_context(|| path.display().to_string())?
    };
    if let Some(bom_path) = bom_path {
        let columns = bom::Columns {
            refdes: config.bom.refdes_column.clone(),
            mpn: config.bom.mpn_column.clone(),
        };
        bom::apply(&mut board, &read_text(bom_path)?, &columns)
            .with_context(|| bom_path.display().to_string())?;
    } else if !is_xml && config.mpn.is_empty() {
        bail!("IPC-D-356A has no part numbers; pass --bom or set [mpn] in the config");
    }
    Ok(board)
}
