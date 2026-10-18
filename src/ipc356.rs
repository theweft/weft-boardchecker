// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! IPC-D-356A reader (bare-board test netlist). Carries nets and pins but no part numbers;
//! combine it with a BOM (see [`crate::bom`]).
//!
//! Fixed columns (1-based): 1-3 record type (317 through-hole, 327 SMD, 367 test point),
//! 4-17 net name, 21-26 reference designator, 28-31 pin. Net names longer than 14 characters
//! are aliased with `P  NNAMEn  <name>` parameter records.

use anyhow::{Result, bail};
use std::collections::BTreeMap;

use crate::board::Board;

/// Parses an IPC-D-356A netlist.
///
/// # Errors
/// Fails if the file contains no connectivity records.
pub fn parse(text: &str) -> Result<Board> {
    let mut aliases: BTreeMap<String, String> = BTreeMap::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix('P') {
            let mut words = rest.split_whitespace();
            if let (Some(key), Some(name)) = (words.next(), words.next())
                && key.starts_with("NNAME")
            {
                aliases.insert(key.to_owned(), name.to_owned());
            }
        }
    }

    let mut board = Board::new();
    let mut records = 0usize;
    for line in text.lines() {
        let op = column(line, 1, 3);
        if !matches!(op.as_str(), "317" | "327" | "367") {
            continue;
        }
        let mut net = column(line, 4, 17);
        let refdes = column(line, 21, 26);
        let pin = column(line, 28, 31);
        if let Some(name) = aliases.get(&net) {
            net.clone_from(name);
        }
        if net.is_empty() || net == "N/C" || refdes.is_empty() || refdes == "VIA" || pin.is_empty()
        {
            continue;
        }
        board.connect(&net, &refdes, &pin);
        records += 1;
    }
    if records == 0 {
        bail!("IPC-D-356A: no component pin records (317/327/367) found");
    }
    Ok(board)
}

/// Characters `from..=to` (1-based), trimmed; empty when the line is shorter.
fn column(line: &str, from: usize, to: usize) -> String {
    line.chars()
        .skip(from.saturating_sub(1))
        .take(to + 1 - from)
        .collect::<String>()
        .trim()
        .to_owned()
}
