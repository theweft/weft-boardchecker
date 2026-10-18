// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! Parts library: per part number, bus pins, address rule and GPIO properties.

use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::BTreeMap;

const BUILTIN: &str = include_str!("../data/parts.toml");

/// Bus a part sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BusKind {
    /// I2C (and SMBus).
    I2c,
    /// SPI.
    Spi,
}

impl std::fmt::Display for BusKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::I2c => "i2c",
            Self::Spi => "spi",
        })
    }
}

/// How an I2C address is set.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum Address {
    /// Always this address.
    Fixed {
        /// The address.
        value: u16,
    },
    /// One pin selects the address by what it connects to (`gnd`, `vdd` or another pin role).
    Strap {
        /// Pin role of the strap pin.
        pin: String,
        /// Level -> address.
        levels: BTreeMap<String, u16>,
    },
    /// Each pin is one address bit (gnd = 0, vdd = 1), most significant first, added to `base`.
    Bits {
        /// Address with all bits 0.
        base: u16,
        /// Pin roles, most significant bit first.
        pins: Vec<String>,
    },
}

/// One library entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Part {
    /// Human-readable name, including the package.
    pub name: String,
    /// Part number prefixes this entry matches.
    pub mpn: Vec<String>,
    /// Device tree compatible strings the part's driver binds to.
    pub compatible: Vec<String>,
    /// Bus kind.
    pub bus: BusKind,
    /// Pin role -> pin number.
    pub pins: BTreeMap<String, String>,
    /// I2C address rule.
    #[serde(default)]
    pub address: Option<Address>,
    /// Device tree property -> pin role.
    #[serde(default)]
    pub gpios: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    #[serde(default)]
    part: Vec<Part>,
}

/// A set of parts; earlier entries win.
#[derive(Debug, Clone, Default)]
pub struct Library {
    parts: Vec<Part>,
}

impl Library {
    /// The built-in library.
    ///
    /// # Errors
    /// Only if the embedded data is invalid (a build defect, covered by tests).
    pub fn builtin() -> Result<Self> {
        let mut library = Self::default();
        library.add_toml(BUILTIN, "built-in parts library")?;
        Ok(library)
    }

    /// Adds entries from TOML text. Added entries take precedence over existing ones.
    ///
    /// # Errors
    /// Fails on invalid TOML or unknown fields.
    pub fn add_toml(&mut self, text: &str, origin: &str) -> Result<()> {
        let file: File = toml::from_str(text).with_context(|| format!("{origin}: invalid"))?;
        let mut parts = file.part;
        parts.append(&mut self.parts);
        self.parts = parts;
        Ok(())
    }

    /// The entry with the longest matching prefix (case-insensitive) for `mpn`.
    pub fn lookup(&self, mpn: &str) -> Option<&Part> {
        let mpn = mpn.to_ascii_uppercase();
        let mut best: Option<(&Part, usize)> = None;
        for part in &self.parts {
            for prefix in &part.mpn {
                let len = prefix.len();
                if mpn.starts_with(&prefix.to_ascii_uppercase())
                    && best.is_none_or(|(_, best_len)| len > best_len)
                {
                    best = Some((part, len));
                }
            }
        }
        best.map(|(part, _)| part)
    }
}
