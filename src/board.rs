// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! Tool-neutral board model: components, their part numbers and which net each pin is on.

use std::collections::BTreeMap;

/// One component pin.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct PinRef {
    /// Reference designator, such as `U3`.
    pub refdes: String,
    /// Pin (pad) number in the component's package, such as `9`.
    pub pin: String,
}

/// One placed component.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Component {
    /// Manufacturer part number, if the export carried one.
    pub mpn: Option<String>,
    /// Pin number -> net name, for connected pins only.
    pub pins: BTreeMap<String, String>,
}

/// A board: components and nets, as read from IPC-2581 or IPC-D-356A.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Board {
    /// Reference designator -> component.
    pub components: BTreeMap<String, Component>,
    /// Net name -> pins on that net.
    pub nets: BTreeMap<String, Vec<PinRef>>,
}

impl Board {
    /// An empty board.
    pub fn new() -> Self {
        Self::default()
    }

    /// Records that `refdes` pin `pin` is on `net`. Creates the component if needed.
    pub fn connect(&mut self, net: &str, refdes: &str, pin: &str) {
        let component = self.components.entry(refdes.to_owned()).or_default();
        component.pins.insert(pin.to_owned(), net.to_owned());
        let pins = self.nets.entry(net.to_owned()).or_default();
        let pin_ref = PinRef {
            refdes: refdes.to_owned(),
            pin: pin.to_owned(),
        };
        if !pins.contains(&pin_ref) {
            pins.push(pin_ref);
        }
    }

    /// Moves `refdes` pin `pin` to `net` (for what-if checks and tests).
    pub fn rewire(&mut self, refdes: &str, pin: &str, net: &str) {
        let old = self.net_of(refdes, pin).map(str::to_owned);
        if let Some(old) = old
            && let Some(pins) = self.nets.get_mut(&old)
        {
            pins.retain(|p| !(p.refdes == refdes && p.pin == pin));
            if pins.is_empty() {
                self.nets.remove(&old);
            }
        }
        self.connect(net, refdes, pin);
    }

    /// Adds a component that may have no connected pins (still needed for part numbers).
    pub fn add_component(&mut self, refdes: &str) {
        self.components.entry(refdes.to_owned()).or_default();
    }

    /// Sets the part number of `refdes`. Creates the component if needed.
    pub fn set_mpn(&mut self, refdes: &str, mpn: &str) {
        let mpn = mpn.trim();
        if !mpn.is_empty() {
            self.components.entry(refdes.to_owned()).or_default().mpn = Some(mpn.to_owned());
        }
    }

    /// The net `refdes` pin `pin` is on.
    pub fn net_of(&self, refdes: &str, pin: &str) -> Option<&str> {
        self.components
            .get(refdes)?
            .pins
            .get(pin)
            .map(String::as_str)
    }

    /// Pins on `net` (empty for an unknown net).
    pub fn pins_on(&self, net: &str) -> &[PinRef] {
        self.nets.get(net).map_or(&[], Vec::as_slice)
    }

    /// Whether `refdes` looks like a resistor: designator `R<digit>…` with two connected pins.
    pub fn is_resistor(&self, refdes: &str) -> bool {
        let mut chars = refdes.chars();
        chars.next() == Some('R')
            && chars.next().is_some_and(|c| c.is_ascii_digit())
            && self
                .components
                .get(refdes)
                .is_some_and(|c| c.pins.len() == 2)
    }

    /// Nets one resistor away from `net`.
    pub fn nets_via_resistor(&self, net: &str) -> Vec<&str> {
        let mut out = Vec::new();
        for pin_ref in self.pins_on(net) {
            if !self.is_resistor(&pin_ref.refdes) {
                continue;
            }
            if let Some(component) = self.components.get(&pin_ref.refdes) {
                for (pin, other) in &component.pins {
                    if *pin != pin_ref.pin && other != net && !out.contains(&other.as_str()) {
                        out.push(other.as_str());
                    }
                }
            }
        }
        out
    }
}
