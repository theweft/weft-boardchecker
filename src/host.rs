// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! Host profiles: which connector pin is which GPIO, and which GPIOs form which bus.

use anyhow::{Context, Result, bail};
use serde::Deserialize;
use std::collections::BTreeMap;

use crate::parts::BusKind;

const BUILTIN: &[(&str, &str)] = &[(
    "raspberry-pi-40pin",
    include_str!("../data/hosts/raspberry-pi-40pin.toml"),
)];

/// A bus on the host connector.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bus {
    /// Overlay target labels that mean this bus; the first is used in messages.
    pub labels: Vec<String>,
    /// Bus kind.
    pub kind: BusKind,
    /// Signal role -> GPIO.
    pub signals: BTreeMap<String, u32>,
    /// SPI chip selects by index.
    #[serde(default)]
    pub cs: Vec<u32>,
}

impl Bus {
    /// Name used in messages.
    pub fn name(&self) -> &str {
        self.labels.first().map_or("?", String::as_str)
    }
}

/// A host profile.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Host {
    /// Profile name.
    pub name: String,
    /// Label of the GPIO controller in overlays.
    pub gpio_label: String,
    /// `#gpio-cells` of that controller.
    pub gpio_cells: usize,
    /// `#interrupt-cells` of that controller.
    pub interrupt_cells: usize,
    /// Connector pins that are ground.
    pub ground_pins: Vec<String>,
    /// Supply name -> connector pins.
    #[serde(default)]
    pub power_pins: BTreeMap<String, Vec<String>>,
    /// Connector pin -> GPIO.
    pub gpio_pins: BTreeMap<String, u32>,
    /// Buses at their pins.
    pub bus: Vec<Bus>,
}

impl Host {
    /// Names of the built-in profiles.
    pub fn builtin_names() -> impl Iterator<Item = &'static str> {
        BUILTIN.iter().map(|(name, _)| *name)
    }

    /// A built-in profile by name.
    ///
    /// # Errors
    /// Fails for an unknown name.
    pub fn builtin(name: &str) -> Result<Self> {
        let Some((_, text)) = BUILTIN.iter().find(|(n, _)| *n == name) else {
            bail!(
                "unknown host profile {name} (built-in: {})",
                Self::builtin_names().collect::<Vec<_>>().join(", ")
            );
        };
        Self::from_toml(text, name)
    }

    /// A profile from TOML text.
    ///
    /// # Errors
    /// Fails on invalid TOML or unknown fields.
    pub fn from_toml(text: &str, origin: &str) -> Result<Self> {
        toml::from_str(text).with_context(|| format!("host profile {origin}: invalid"))
    }

    /// The bus an overlay label refers to.
    pub fn bus_by_label(&self, label: &str) -> Option<usize> {
        self.bus
            .iter()
            .position(|b| b.labels.iter().any(|l| l == label))
    }
}
