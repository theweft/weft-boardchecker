// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! IPC-2581 (revision B or C) reader: components, logical nets and part numbers.
//!
//! Part numbers come from, in order: the approved vendor list (`Avl/AvlItem/AvlVmpn/AvlMpn`, where
//! KiCad and others put the manufacturer part number), then a BOM characteristic named like a part
//! number (`MPN`, `Manufacturer Part Number`, …). Element names are matched without namespace.

use anyhow::{Context, Result, bail};
use roxmltree::{Document, Node};
use std::collections::BTreeMap;

use crate::board::Board;

/// Parses an IPC-2581 XML document.
///
/// # Errors
/// Fails on malformed XML, a root element other than `IPC-2581`, or a file without any net data.
pub fn parse(xml: &str) -> Result<Board> {
    let doc = Document::parse(xml).context("IPC-2581: not well-formed XML")?;
    let root = doc.root_element();
    if root.tag_name().name() != "IPC-2581" {
        bail!(
            "IPC-2581: root element is <{}>, expected <IPC-2581>",
            root.tag_name().name()
        );
    }

    let mut board = Board::new();
    // Component@part -> refdes list, to attach part numbers later.
    let mut by_part: BTreeMap<String, Vec<String>> = BTreeMap::new();

    for component in elements(root, "Component") {
        let Some(refdes) = component.attribute("refDes") else {
            continue;
        };
        board.add_component(refdes);
        if let Some(part) = component.attribute("part") {
            by_part
                .entry(part.to_owned())
                .or_default()
                .push(refdes.to_owned());
        }
    }

    let mut have_nets = false;
    for net in elements(root, "LogicalNet") {
        let Some(name) = net.attribute("name") else {
            continue;
        };
        for pin_ref in net.children().filter(|n| is(*n, "PinRef")) {
            if let (Some(refdes), Some(pin)) =
                (pin_ref.attribute("componentRef"), pin_ref.attribute("pin"))
            {
                board.connect(name, refdes, pin);
                have_nets = true;
            }
        }
    }

    // Fallback when the logical netlist was not exported: copper features grouped in
    // <Set net="…"> elements that contain pads with <PinRef>.
    if !have_nets {
        for set in elements(root, "Set") {
            let Some(name) = set.attribute("net") else {
                continue;
            };
            for pin_ref in set.descendants().filter(|n| is(*n, "PinRef")) {
                if let (Some(refdes), Some(pin)) =
                    (pin_ref.attribute("componentRef"), pin_ref.attribute("pin"))
                {
                    board.connect(name, refdes, pin);
                    have_nets = true;
                }
            }
        }
    }
    if !have_nets {
        bail!("IPC-2581: no nets found (export with the logical net section enabled)");
    }

    attach_part_numbers(root, &mut board, &mut by_part);
    Ok(board)
}

/// Sets part numbers from the approved vendor list, falling back to BOM characteristics.
fn attach_part_numbers(
    root: Node<'_, '_>,
    board: &mut Board,
    by_part: &mut BTreeMap<String, Vec<String>>,
) {
    let mut mpn_by_part: BTreeMap<String, String> = BTreeMap::new();
    for item in elements(root, "AvlItem") {
        let Some(part) = item.attribute("OEMDesignNumber") else {
            continue;
        };
        let mpn = item
            .descendants()
            .find(|n| is(*n, "AvlMpn"))
            .and_then(|n| n.attribute("name"));
        if let Some(mpn) = mpn {
            mpn_by_part
                .entry(part.to_owned())
                .or_insert_with(|| mpn.to_owned());
        }
    }
    for item in elements(root, "BomItem") {
        let Some(part) = item.attribute("OEMDesignNumberRef") else {
            continue;
        };
        for refdes in item.children().filter(|n| is(*n, "RefDes")) {
            if let Some(name) = refdes.attribute("name") {
                let refs = by_part.entry(part.to_owned()).or_default();
                if !refs.iter().any(|r| r == name) {
                    refs.push(name.to_owned());
                }
            }
        }
        if mpn_by_part.contains_key(part) {
            continue;
        }
        let mpn = item
            .descendants()
            .filter(|n| is(*n, "Textual"))
            .find_map(|t| {
                let name = t.attribute("textualCharacteristicName")?;
                let value = t.attribute("textualCharacteristicValue")?;
                is_mpn_field(name).then_some(value)
            });
        if let Some(mpn) = mpn {
            mpn_by_part.insert(part.to_owned(), mpn.to_owned());
        }
    }
    for (part, mpn) in &mpn_by_part {
        for refdes in by_part.get(part).into_iter().flatten() {
            board.set_mpn(refdes, mpn);
        }
    }
}

/// Whether a field name means "manufacturer part number".
pub fn is_mpn_field(name: &str) -> bool {
    let key: String = name
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect();
    matches!(
        key.as_str(),
        "mpn" | "manufacturerpartnumber" | "manufacturerpn" | "mfrpn" | "mfgpn" | "mfrpartnumber"
    )
}

fn is(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

fn elements<'a, 'i>(root: Node<'a, 'i>, name: &'a str) -> impl Iterator<Item = Node<'a, 'i>> {
    root.descendants().filter(move |n| is(*n, name))
}
