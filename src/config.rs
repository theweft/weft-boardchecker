// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! Per-board configuration file (`boardcheck.toml`).

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// A bus device that the overlay intentionally does not describe.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Expect {
    /// Reference designator.
    pub refdes: String,
    /// Why it is not in the overlay.
    pub reason: String,
}

/// BOM column names (IPC-D-356A input only).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BomConfig {
    /// Designator column header.
    pub refdes_column: Option<String>,
    /// Part number column header.
    pub mpn_column: Option<String>,
}

/// The configuration file.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Built-in host profile name or path to a profile file.
    pub host: Option<String>,
    /// Reference designator of the host connector on this board.
    pub connector: Option<String>,
    /// Extra chip files (weft-parts format), relative to the config file.
    #[serde(default)]
    pub parts: Vec<PathBuf>,
    /// Part number overrides by designator, for exports without part numbers.
    #[serde(default)]
    pub mpn: BTreeMap<String, String>,
    /// Bus devices not described by the overlay.
    #[serde(default)]
    pub expect: Vec<Expect>,
    /// Designators to skip entirely.
    #[serde(default)]
    pub ignore: Vec<String>,
    /// BOM column names.
    #[serde(default)]
    pub bom: BomConfig,
    /// Directory relative paths resolve against.
    #[serde(skip)]
    pub base: PathBuf,
}

impl Config {
    /// Loads a config file.
    ///
    /// # Errors
    /// Fails if the file cannot be read or is invalid.
    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path)
            .with_context(|| format!("cannot read {}", path.display()))?;
        let mut config: Self =
            toml::from_str(&text).with_context(|| format!("{}: invalid", path.display()))?;
        config.base = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Ok(config)
    }

    /// Resolves a path from the config file against its directory.
    pub fn resolve(&self, path: &Path) -> PathBuf {
        if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.base.join(path)
        }
    }
}
