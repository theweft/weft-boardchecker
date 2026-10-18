// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! CSV bill of materials: adds part numbers to a board read from IPC-D-356A.

use anyhow::{Context, Result, bail};

use crate::board::Board;
use crate::ipc2581::is_mpn_field;

/// Column names to use instead of auto-detection.
#[derive(Debug, Clone, Default)]
pub struct Columns {
    /// Header of the reference designator column.
    pub refdes: Option<String>,
    /// Header of the manufacturer part number column.
    pub mpn: Option<String>,
}

/// Applies part numbers from a CSV BOM. Designator cells may list several designators separated
/// by commas, semicolons or spaces, and ranges such as `R1-R4`.
///
/// # Errors
/// Fails on malformed CSV or when a needed column cannot be found.
pub fn apply(board: &mut Board, csv_text: &str, columns: &Columns) -> Result<usize> {
    let mut reader = csv::ReaderBuilder::new()
        .flexible(true)
        .from_reader(csv_text.as_bytes());
    let headers = reader
        .headers()
        .context("BOM: cannot read the header row")?
        .clone();
    let find = |wanted: Option<&String>, auto: &dyn Fn(&str) -> bool, what: &str| {
        let index = match wanted {
            Some(name) => headers
                .iter()
                .position(|h| h.trim().eq_ignore_ascii_case(name)),
            None => headers.iter().position(|h| auto(h.trim())),
        };
        index.with_context(|| {
            format!(
                "BOM: no {what} column (headers: {}); set it in the config",
                headers.iter().collect::<Vec<_>>().join(", ")
            )
        })
    };
    let refdes_col = find(columns.refdes.as_ref(), &is_refdes_field, "designator")?;
    let mpn_col = find(columns.mpn.as_ref(), &is_mpn_field, "part number")?;

    let mut applied = 0usize;
    for record in reader.records() {
        let record = record.context("BOM: malformed row")?;
        let (Some(refs), Some(mpn)) = (record.get(refdes_col), record.get(mpn_col)) else {
            continue;
        };
        for refdes in expand(refs)? {
            board.set_mpn(&refdes, mpn);
            applied += 1;
        }
    }
    Ok(applied)
}

fn is_refdes_field(name: &str) -> bool {
    let key = name.to_ascii_lowercase();
    matches!(
        key.as_str(),
        "reference" | "references" | "designator" | "designators" | "refdes" | "ref" | "ref des"
    )
}

/// Splits a designator cell and expands ranges like `R1-R4`.
fn expand(cell: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    for item in cell
        .split([',', ';', ' ', '\t'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        let Some((first, last)) = item.split_once('-') else {
            out.push(item.to_owned());
            continue;
        };
        let (Some((prefix_a, a)), Some((prefix_b, b))) = (split_number(first), split_number(last))
        else {
            out.push(item.to_owned());
            continue;
        };
        if prefix_a != prefix_b || a > b || b - a > 10_000 {
            bail!("BOM: cannot expand designator range {item}");
        }
        out.extend((a..=b).map(|n| format!("{prefix_a}{n}")));
    }
    Ok(out)
}

fn split_number(s: &str) -> Option<(&str, u32)> {
    let digits = s.len() - s.trim_end_matches(|c: char| c.is_ascii_digit()).len();
    let (prefix, number) = s.split_at(s.len() - digits);
    if prefix.is_empty() || number.is_empty() {
        return None;
    }
    Some((prefix, number.parse().ok()?))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::expand;

    #[test]
    fn expands_lists_and_ranges() {
        assert_eq!(
            expand("R1, R3-R5;C2").unwrap(),
            ["R1", "R3", "R4", "R5", "C2"]
        );
    }
}
