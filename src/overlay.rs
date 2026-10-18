// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! Device tree overlay (compiled with `dtc -@`): fragments, their targets and unresolved phandle
//! references, which `__fixups__` records as `label = "/path:property:byte-offset"`.

use anyhow::Result;
use std::collections::BTreeMap;

use crate::dtb::{self, Node};

/// What a fragment applies to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// `target = <&label>`.
    Label(String),
    /// `target-path = "/…"`.
    Path(String),
    /// Neither could be resolved.
    Unknown,
}

/// One overlay fragment.
#[derive(Debug, Clone)]
pub struct Fragment {
    /// Path of the `__overlay__` node, such as `/fragment@0/__overlay__`.
    pub path: String,
    /// What the fragment applies to.
    pub target: Target,
    /// The `__overlay__` node: properties and children merged into the target.
    pub node: Node,
}

/// A parsed overlay.
#[derive(Debug, Clone, Default)]
pub struct Overlay {
    /// Fragments in blob order.
    pub fragments: Vec<Fragment>,
    /// (node path, property, byte offset) -> label of the referenced node.
    fixups: BTreeMap<(String, String, usize), String>,
}

impl Overlay {
    /// Parses a compiled overlay.
    ///
    /// # Errors
    /// Fails if the blob is not a valid device tree.
    pub fn parse(blob: &[u8]) -> Result<Self> {
        let root = dtb::parse(blob)?;
        let mut fixups = BTreeMap::new();
        if let Some(node) = root.child("__fixups__") {
            for (label, value) in &node.props {
                for entry in dtb::strings(value) {
                    // "/fragment@0/__overlay__/adc@48:interrupt-parent:0"
                    let mut parts = entry.rsplitn(3, ':');
                    let (Some(offset), Some(prop), Some(path)) =
                        (parts.next(), parts.next(), parts.next())
                    else {
                        continue;
                    };
                    if let Ok(offset) = offset.parse::<usize>() {
                        fixups.insert((path.to_owned(), prop.to_owned(), offset), label.clone());
                    }
                }
            }
        }

        let mut fragments = Vec::new();
        for fragment in &root.children {
            let Some(overlay) = fragment.child("__overlay__") else {
                continue;
            };
            let fragment_path = format!("/{}", fragment.name);
            let target =
                if let Some(label) = fixups.get(&(fragment_path.clone(), "target".to_owned(), 0)) {
                    Target::Label(label.clone())
                } else if let Some(path) = fragment.string("target-path") {
                    Target::Path(path)
                } else {
                    Target::Unknown
                };
            fragments.push(Fragment {
                path: format!("{fragment_path}/__overlay__"),
                target,
                node: overlay.clone(),
            });
        }
        Ok(Self { fragments, fixups })
    }

    /// Label referenced by cell `cell` of property `prop` of the node at `path`, if that cell is an
    /// unresolved phandle reference.
    pub fn label_at(&self, path: &str, prop: &str, cell: usize) -> Option<&str> {
        self.fixups
            .get(&(path.to_owned(), prop.to_owned(), cell * 4))
            .map(String::as_str)
    }
}
