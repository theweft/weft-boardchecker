// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! The cross-check: board devices on host buses versus overlay nodes.
//!
//! 1. Every part from the library is traced to a host bus through the connector (directly or
//!    through one resistor), and its address or chip select is derived from the netlist.
//! 2. Every overlay node with a `reg` under a fragment that targets a host bus is collected.
//! 3. Both sides are matched by (bus, address or chip select); compatibles and GPIO properties of
//!    matched pairs are compared. Anything unmatched, duplicated or miswired is reported.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use crate::board::{Board, Component};
use crate::dtb::{self, Node};
use crate::host::Host;
use crate::overlay::{Overlay, Target};
use weft_parts::{Address, BusKind, Library, Part};

/// How bad a finding is. Ordered from worst to best.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    /// The board and the overlay disagree, or the board is miswired.
    Error,
    /// Something could not be checked.
    Warning,
    /// Context, such as skipped fragments.
    Info,
    /// A check that passed.
    Ok,
}

/// One result line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Severity.
    pub severity: Severity,
    /// Designator or overlay path the finding is about.
    pub subject: String,
    /// What was found, as a sentence.
    pub message: String,
}

/// Everything the check needs.
#[derive(Debug, Clone, Copy)]
pub struct Input<'a> {
    /// The board.
    pub board: &'a Board,
    /// The overlay.
    pub overlay: &'a Overlay,
    /// Host profile.
    pub host: &'a Host,
    /// Parts library.
    pub parts: &'a Library,
    /// Designator of the host connector.
    pub connector: &'a str,
    /// Designator -> reason, for bus devices the overlay intentionally omits.
    pub expect: &'a BTreeMap<String, String>,
    /// Designators to skip.
    pub ignore: &'a BTreeSet<String>,
}

/// Runs all checks. Findings are sorted worst first, then by subject.
pub fn check(input: Input<'_>) -> Vec<Finding> {
    let mut ctx = Ctx::new(input);
    ctx.run();
    let mut findings = ctx.findings;
    findings.sort_by(|a, b| (a.severity, &a.subject).cmp(&(b.severity, &b.subject)));
    findings
}

struct BoardDevice<'a> {
    refdes: &'a str,
    mpn: &'a str,
    part: Part<'a>,
    bus: usize,
    reg: u32,
}

struct OverlayDevice<'a> {
    bus: usize,
    reg: u32,
    path: String,
    node: &'a Node,
}

struct Ctx<'a> {
    input: Input<'a>,
    findings: Vec<Finding>,
    /// Net -> GPIOs of connector pins on that net.
    gpios_by_net: BTreeMap<&'a str, BTreeSet<u32>>,
    ground_nets: BTreeSet<&'a str>,
    power_nets: BTreeSet<&'a str>,
    /// Effective chip-select GPIOs per SPI bus (host default, or cs-gpios from the overlay).
    chip_selects: Vec<Vec<u32>>,
}

impl<'a> Ctx<'a> {
    fn new(input: Input<'a>) -> Self {
        let mut gpios_by_net: BTreeMap<&str, BTreeSet<u32>> = BTreeMap::new();
        let mut ground_nets = BTreeSet::new();
        let mut power_nets = BTreeSet::new();
        if let Some(connector) = input.board.components.get(input.connector) {
            for (pin, net) in &connector.pins {
                if let Some(gpio) = input.host.gpio_pins.get(pin) {
                    gpios_by_net.entry(net.as_str()).or_default().insert(*gpio);
                }
                if input.host.ground_pins.contains(pin) {
                    ground_nets.insert(net.as_str());
                }
                if input
                    .host
                    .power_pins
                    .values()
                    .any(|pins| pins.contains(pin))
                {
                    power_nets.insert(net.as_str());
                }
            }
        }
        let chip_selects = input.host.bus.iter().map(|b| b.cs.clone()).collect();
        Self {
            input,
            findings: Vec::new(),
            gpios_by_net,
            ground_nets,
            power_nets,
            chip_selects,
        }
    }

    fn push(&mut self, severity: Severity, subject: &str, message: String) {
        self.findings.push(Finding {
            severity,
            subject: subject.to_owned(),
            message,
        });
    }

    fn run(&mut self) {
        let input = self.input;
        if !input.board.components.contains_key(input.connector) {
            self.push(
                Severity::Error,
                input.connector,
                format!(
                    "The host connector {} is not on the board; set `connector` to its designator.",
                    input.connector
                ),
            );
            return;
        }
        let overlay_devices = self.overlay_devices();
        let board_devices = self.board_devices();
        self.duplicates(
            board_devices
                .iter()
                .map(|d| (d.bus, d.reg, d.refdes.to_owned())),
            "share",
        );
        self.duplicates(
            overlay_devices
                .iter()
                .map(|d| (d.bus, d.reg, d.path.clone())),
            "both claim",
        );
        self.match_devices(&board_devices, &overlay_devices);
        for refdes in input.expect.keys() {
            if !input.board.components.contains_key(refdes) {
                self.push(
                    Severity::Warning,
                    refdes,
                    format!("{refdes} is listed under `expect` but is not on the board."),
                );
            }
        }
    }

    // ----- overlay side -------------------------------------------------------------------

    fn overlay_devices(&mut self) -> Vec<OverlayDevice<'a>> {
        let input = self.input;
        let host = input.host;
        let mut out = Vec::new();
        for fragment in &input.overlay.fragments {
            let label = match &fragment.target {
                Target::Label(label) => label,
                Target::Path(path) => {
                    self.push(
                        Severity::Info,
                        &fragment.path,
                        format!("Targets the path {path}; only label targets are checked."),
                    );
                    continue;
                }
                Target::Unknown => {
                    self.push(
                        Severity::Warning,
                        &fragment.path,
                        "Fragment target could not be resolved (compile with `dtc -@`).".to_owned(),
                    );
                    continue;
                }
            };
            let Some(bus) = host.bus_by_label(label) else {
                self.push(
                    Severity::Info,
                    &fragment.path,
                    format!("Targets &{label}, which is not a host bus; not checked."),
                );
                continue;
            };
            if is_disabled(&fragment.node) {
                continue;
            }
            if let Some(cs) = self.cs_override(&fragment.path, &fragment.node, bus)
                && let Some(slot) = self.chip_selects.get_mut(bus)
            {
                *slot = cs;
            }
            for child in &fragment.node.children {
                let Some(reg) = child.cells("reg").first().copied() else {
                    continue;
                };
                if is_disabled(child) {
                    continue;
                }
                out.push(OverlayDevice {
                    bus,
                    reg,
                    path: format!("{}/{}", fragment.path, child.name),
                    node: child,
                });
            }
        }
        out
    }

    /// Chip selects from `cs-gpios` on an SPI bus fragment. A zero phandle keeps the native CS.
    fn cs_override(&self, path: &str, node: &Node, bus: usize) -> Option<Vec<u32>> {
        let host = self.input.host;
        let cells = node.cells("cs-gpios");
        if cells.is_empty() {
            return None;
        }
        let native = host.bus.get(bus).map(|b| b.cs.clone()).unwrap_or_default();
        let mut out = Vec::new();
        let mut index = 0usize;
        let mut cell = 0usize;
        while let Some(phandle) = cells.get(cell) {
            if *phandle == 0 {
                out.extend(native.get(index).copied());
                cell += 1;
            } else {
                if self.input.overlay.label_at(path, "cs-gpios", cell) == Some(&host.gpio_label)
                    && let Some(gpio) = cells.get(cell + 1)
                {
                    out.push(*gpio);
                }
                cell += 1 + host.gpio_cells;
            }
            index += 1;
        }
        Some(out)
    }

    // ----- board side ---------------------------------------------------------------------

    fn board_devices(&mut self) -> Vec<BoardDevice<'a>> {
        let input = self.input;
        let mut out = Vec::new();
        for (refdes, component) in &input.board.components {
            if refdes == input.connector || input.ignore.contains(refdes) {
                continue;
            }
            let part = component.mpn.as_deref().and_then(|m| input.parts.lookup(m));
            let Some(part) = part else {
                self.unknown_part(refdes, component);
                continue;
            };
            let mpn = component.mpn.as_deref().unwrap_or_default();
            let located = match part.chip.bus {
                BusKind::I2c => self.locate_i2c(refdes, component, &part),
                BusKind::Spi => self.locate_spi(refdes, &part),
            };
            match located {
                Ok(Some((bus, reg))) => out.push(BoardDevice {
                    refdes,
                    mpn,
                    part,
                    bus,
                    reg,
                }),
                Ok(None) => {}
                Err(message) => self.push(Severity::Error, refdes, message),
            }
        }
        out
    }

    /// Warns about components outside the library that touch a host bus signal.
    fn unknown_part(&mut self, refdes: &str, component: &Component) {
        if component.pins.len() <= 2 || self.input.expect.contains_key(refdes) {
            return;
        }
        for (pin, net) in &component.pins {
            for gpio in self.gpios(net) {
                if let Some(what) = self.bus_signal(gpio) {
                    let mpn = component.mpn.as_deref().unwrap_or("no part number");
                    self.push(
                        Severity::Warning,
                        refdes,
                        format!(
                            "{refdes} ({mpn}) pin {pin} is on {what}, but the part is not in the \
                             parts library; add it, or list it under `ignore`."
                        ),
                    );
                    return;
                }
            }
        }
    }

    /// "i2c1 SDA" for a GPIO that is a bus signal or chip select.
    fn bus_signal(&self, gpio: u32) -> Option<String> {
        for (index, bus) in self.input.host.bus.iter().enumerate() {
            for (role, g) in &bus.signals {
                if *g == gpio {
                    return Some(format!("{} {}", bus.name(), role.to_uppercase()));
                }
            }
            let cs = self.chip_selects.get(index).map_or(&bus.cs, |c| c);
            if let Some(n) = cs.iter().position(|g| *g == gpio) {
                return Some(format!("{} chip select {n}", bus.name()));
            }
        }
        None
    }

    fn pin_net(&self, refdes: &str, part: &Part<'_>, role: &str) -> Result<&'a str, String> {
        let pin = part
            .pin(role)
            .ok_or_else(|| format!("Chip file \"{}\" has no `{role}` pin.", part.name()))?;
        self.input
            .board
            .net_of(refdes, pin)
            .ok_or_else(|| format!("{refdes} pin {pin} ({role}) is not connected."))
    }

    fn locate_i2c(
        &mut self,
        refdes: &str,
        component: &Component,
        part: &Part<'_>,
    ) -> Result<Option<(usize, u32)>, String> {
        let sda = self.gpios(self.pin_net(refdes, part, "sda")?);
        let scl = self.gpios(self.pin_net(refdes, part, "scl")?);
        let mut found = None;
        for (index, bus) in self.input.host.bus.iter().enumerate() {
            if bus.kind != BusKind::I2c {
                continue;
            }
            let (Some(bus_sda), Some(bus_scl)) = (bus.signals.get("sda"), bus.signals.get("scl"))
            else {
                continue;
            };
            if sda.contains(bus_sda) && scl.contains(bus_scl) {
                found = Some(index);
                break;
            }
            if sda.contains(bus_scl) && scl.contains(bus_sda) {
                return Err(format!(
                    "{refdes} has SDA and SCL swapped on {}.",
                    bus.name()
                ));
            }
        }
        let Some(bus) = found else {
            return self.not_on_bus(refdes, "I2C", &sda, &scl);
        };
        let address = self.address(refdes, component, part)?;
        Ok(Some((bus, u32::from(address))))
    }

    fn locate_spi(
        &mut self,
        refdes: &str,
        part: &Part<'_>,
    ) -> Result<Option<(usize, u32)>, String> {
        let host = self.input.host;
        let sck = self.gpios(self.pin_net(refdes, part, "sck")?);
        let cs = self.gpios(self.pin_net(refdes, part, "cs")?);
        let found = host.bus.iter().enumerate().find(|(_, b)| {
            b.kind == BusKind::Spi && b.signals.get("sck").is_some_and(|g| sck.contains(g))
        });
        let Some((index, bus)) = found else {
            return self.not_on_bus(refdes, "SPI", &sck, &cs);
        };
        // mosi is the device input, miso its output; either may be absent on simplex parts.
        for (role, other) in [("mosi", "miso"), ("miso", "mosi")] {
            if !part.package.pins.contains_key(role) {
                continue;
            }
            let gpios = self.gpios(self.pin_net(refdes, part, role)?);
            let expected = bus.signals.get(role);
            if expected.is_some_and(|g| gpios.contains(g)) {
                continue;
            }
            if bus.signals.get(other).is_some_and(|g| gpios.contains(g)) {
                return Err(format!(
                    "{refdes} has MOSI and MISO swapped on {}.",
                    bus.name()
                ));
            }
            return Err(format!(
                "{refdes} {} reaches {}, but {} {} is GPIO {}.",
                role.to_uppercase(),
                describe_gpios(&gpios),
                bus.name(),
                role.to_uppercase(),
                expected.map_or("?".to_owned(), u32::to_string)
            ));
        }
        let selects = self.chip_selects.get(index).cloned().unwrap_or_default();
        let position = selects.iter().position(|g| cs.contains(g));
        let Some(position) = position else {
            return Err(format!(
                "{refdes} chip select reaches {}, which is not a chip select of {} ({}); \
                 add it to cs-gpios in the overlay.",
                describe_gpios(&cs),
                bus.name(),
                describe_gpios(&selects.iter().copied().collect())
            ));
        };
        Ok(Some((index, u32::try_from(position).unwrap_or(u32::MAX))))
    }

    fn not_on_bus(
        &mut self,
        refdes: &str,
        kind: &str,
        a: &BTreeSet<u32>,
        b: &BTreeSet<u32>,
    ) -> Result<Option<(usize, u32)>, String> {
        if a.is_empty() && b.is_empty() {
            self.push(
                Severity::Warning,
                refdes,
                format!(
                    "{refdes} is an {kind} part but is not wired to the host connector (behind a \
                     mux or level shifter?); not checked."
                ),
            );
            return Ok(None);
        }
        Err(format!(
            "{refdes} {kind} lines reach {} and {}, which are not a host {kind} bus.",
            describe_gpios(a),
            describe_gpios(b)
        ))
    }

    // ----- addresses ----------------------------------------------------------------------

    fn address(&self, refdes: &str, component: &Component, part: &Part<'_>) -> Result<u16, String> {
        let Some(rule) = &part.chip.address else {
            return Err(format!(
                "Chip file \"{}\" has no address rule.",
                part.name()
            ));
        };
        match rule {
            Address::Fixed { value } => Ok(*value),
            Address::Strap { pin, levels } => {
                let net = self.pin_net(refdes, part, pin)?;
                let found = self.levels(net, refdes, component, part, pin);
                let addresses: BTreeSet<u16> = found
                    .iter()
                    .filter_map(|l| levels.get(l))
                    .copied()
                    .collect();
                let mut iter = addresses.iter();
                match (iter.next(), iter.next()) {
                    (Some(address), None) => Ok(*address),
                    (None, _) => Err(format!(
                        "{refdes} address pin ({pin}) is on net {net}, which is none of {}; the \
                         address is undefined.",
                        levels.keys().cloned().collect::<Vec<_>>().join(", ")
                    )),
                    (Some(_), Some(_)) => Err(format!(
                        "{refdes} address pin ({pin}) is tied to several levels ({}).",
                        found.into_iter().collect::<Vec<_>>().join(", ")
                    )),
                }
            }
            Address::Bits { base, pins } => {
                let mut address = *base;
                for (bit, pin) in pins.iter().rev().enumerate() {
                    let net = self.pin_net(refdes, part, pin)?;
                    let found = self.levels(net, refdes, component, part, pin);
                    match (found.contains("gnd"), found.contains("vdd")) {
                        (true, false) => {}
                        (false, true) => address |= 1 << bit,
                        _ => {
                            return Err(format!(
                                "{refdes} address pin {pin} is on net {net}, which is neither \
                                 ground nor supply."
                            ));
                        }
                    }
                }
                Ok(address)
            }
        }
    }

    /// Levels a strap net is tied to: `gnd`, `vdd` or other pin roles of the same part.
    /// Looks through one resistor if nothing is connected directly.
    fn levels(
        &self,
        net: &str,
        refdes: &str,
        component: &Component,
        part: &Part<'_>,
        own_role: &str,
    ) -> BTreeSet<String> {
        let direct = |net: &str| {
            let mut out = BTreeSet::new();
            let part_net = |role: &str| {
                part.package
                    .pins
                    .get(role)
                    .and_then(|p| component.pins.get(p))
            };
            if self.ground_nets.contains(net) || part_net("gnd").is_some_and(|n| n == net) {
                out.insert("gnd".to_owned());
            }
            if self.power_nets.contains(net) || part_net("vdd").is_some_and(|n| n == net) {
                out.insert("vdd".to_owned());
            }
            for (role, pin) in &part.package.pins {
                if role != own_role
                    && role != "gnd"
                    && role != "vdd"
                    && self.input.board.net_of(refdes, pin) == Some(net)
                {
                    out.insert(role.clone());
                }
            }
            out
        };
        let found = direct(net);
        if !found.is_empty() {
            return found;
        }
        self.input
            .board
            .nets_via_resistor(net)
            .into_iter()
            .flat_map(direct)
            .collect()
    }

    /// GPIOs a net reaches: connector pins on the net, or else one resistor away.
    fn gpios(&self, net: &str) -> BTreeSet<u32> {
        if let Some(direct) = self.gpios_by_net.get(net) {
            return direct.clone();
        }
        self.input
            .board
            .nets_via_resistor(net)
            .into_iter()
            .filter_map(|n| self.gpios_by_net.get(n))
            .flatten()
            .copied()
            .collect()
    }

    // ----- matching -----------------------------------------------------------------------

    fn where_(&self, bus: usize, reg: u32) -> String {
        let Some(b) = self.input.host.bus.get(bus) else {
            return "?".to_owned();
        };
        match b.kind {
            BusKind::I2c => format!("{} address 0x{reg:02x}", b.name()),
            BusKind::Spi => format!("{} chip select {reg}", b.name()),
        }
    }

    fn duplicates(&mut self, items: impl Iterator<Item = (usize, u32, String)>, verb: &str) {
        let mut seen: BTreeMap<(usize, u32), Vec<String>> = BTreeMap::new();
        for (bus, reg, who) in items {
            seen.entry((bus, reg)).or_default().push(who);
        }
        for ((bus, reg), who) in seen {
            if who.len() > 1 {
                let place = self.where_(bus, reg);
                let subject = who.first().cloned().unwrap_or_default();
                self.push(
                    Severity::Error,
                    &subject,
                    format!("{} {verb} {place}.", who.join(" and ")),
                );
            }
        }
    }

    fn match_devices(&mut self, board: &[BoardDevice<'a>], overlay: &[OverlayDevice<'a>]) {
        let input = self.input;
        for dev in board {
            let place = self.where_(dev.bus, dev.reg);
            let label = format!("{} ({})", dev.refdes, dev.mpn);
            let node = overlay
                .iter()
                .find(|o| o.bus == dev.bus && o.reg == dev.reg);
            let Some(node) = node else {
                if let Some(reason) = input.expect.get(dev.refdes) {
                    self.push(
                        Severity::Ok,
                        dev.refdes,
                        format!("{label} is at {place}; not in the overlay by design ({reason})."),
                    );
                    continue;
                }
                let mut message =
                    format!("{label} is at {place}, but the overlay has no node there.");
                for other in overlay
                    .iter()
                    .filter(|o| o.bus == dev.bus && o.reg != dev.reg)
                {
                    let compatible = other.node.strings("compatible");
                    if compatibles(&dev.part)
                        .iter()
                        .any(|c| compatible.contains(c))
                    {
                        let _ = write!(
                            message,
                            " The overlay has {} at {}; check the address strap.",
                            other.path,
                            self.where_(other.bus, other.reg)
                        );
                    }
                }
                self.push(Severity::Error, dev.refdes, message);
                continue;
            };
            let compatible = node.node.strings("compatible");
            if compatibles(&dev.part)
                .iter()
                .any(|c| compatible.contains(c))
            {
                self.push(
                    Severity::Ok,
                    dev.refdes,
                    format!("{label} at {place} matches {}.", node.path),
                );
            } else {
                self.push(
                    Severity::Error,
                    dev.refdes,
                    format!(
                        "{label} is at {place}, but {} there is compatible \"{}\" (expected \"{}\").",
                        node.path,
                        compatible.join("\", \""),
                        compatibles(&dev.part).join("\", \"")
                    ),
                );
            }
            if input.expect.contains_key(dev.refdes) {
                self.push(
                    Severity::Info,
                    dev.refdes,
                    format!(
                        "{} is listed under `expect` but the overlay describes it.",
                        dev.refdes
                    ),
                );
            }
            self.check_gpios(dev, node);
        }
        for node in overlay {
            if !board.iter().any(|d| d.bus == node.bus && d.reg == node.reg) {
                let place = self.where_(node.bus, node.reg);
                self.push(
                    Severity::Error,
                    &node.path,
                    format!(
                        "The overlay declares \"{}\" at {place}, but no device on the board is there.",
                        node.node.strings("compatible").join("\", \"")
                    ),
                );
            }
        }
    }

    // ----- GPIO properties ----------------------------------------------------------------

    fn check_gpios(&mut self, dev: &BoardDevice<'a>, node: &OverlayDevice<'a>) {
        let input = self.input;
        let host = input.host;
        let overlay = input.overlay;
        for (prop, value) in &node.node.props {
            let is_gpio = prop == "gpios" || prop.ends_with("-gpios") || prop.ends_with("-gpio");
            if !is_gpio {
                continue;
            }
            let cells = dtb::cells(value);
            match overlay.label_at(&node.path, prop, 0) {
                Some(label) if label == host.gpio_label => {
                    if let Some(gpio) = cells.get(1) {
                        self.check_pin(dev, prop, *gpio);
                    }
                }
                Some(label) => self.push(
                    Severity::Info,
                    dev.refdes,
                    format!(
                        "{prop} refers to &{label}, not the host GPIO controller; not checked."
                    ),
                ),
                None => {}
            }
        }
        let extended = node.node.cells("interrupts-extended");
        let gpio = if extended.is_empty() {
            let parent = overlay.label_at(&node.path, "interrupt-parent", 0);
            if parent == Some(host.gpio_label.as_str()) {
                node.node.cells("interrupts").first().copied()
            } else {
                None
            }
        } else if overlay.label_at(&node.path, "interrupts-extended", 0)
            == Some(host.gpio_label.as_str())
        {
            extended.get(1).copied()
        } else {
            None
        };
        if let Some(gpio) = gpio {
            self.check_pin(dev, "interrupts", gpio);
        }
    }

    fn check_pin(&mut self, dev: &BoardDevice<'a>, prop: &str, gpio: u32) {
        let Some(role) = dev
            .part
            .chip
            .devicetree
            .as_ref()
            .and_then(|dt| dt.gpios.get(prop))
        else {
            self.push(
                Severity::Warning,
                dev.refdes,
                format!(
                    "{prop} uses GPIO {gpio}, but the parts library does not say which pin of {} \
                     that is; not checked.",
                    dev.mpn
                ),
            );
            return;
        };
        let net = match self.pin_net(dev.refdes, &dev.part, role) {
            Ok(net) => net,
            Err(message) => {
                self.push(
                    Severity::Error,
                    dev.refdes,
                    format!("{prop} uses GPIO {gpio}, but {message}"),
                );
                return;
            }
        };
        let pin = dev.part.package.pins.get(role).cloned().unwrap_or_default();
        let gpios = self.gpios(net);
        if gpios.contains(&gpio) {
            self.push(
                Severity::Ok,
                dev.refdes,
                format!(
                    "{} {prop}: GPIO {gpio} reaches pin {pin} ({role}).",
                    dev.refdes
                ),
            );
        } else if gpios.is_empty() {
            self.push(
                Severity::Error,
                dev.refdes,
                format!(
                    "{prop} uses GPIO {gpio}, but {} pin {pin} ({role}) is not wired to the host \
                     connector.",
                    dev.refdes
                ),
            );
        } else {
            self.push(
                Severity::Error,
                dev.refdes,
                format!(
                    "{prop} uses GPIO {gpio}, but {} pin {pin} ({role}) is wired to {}.",
                    dev.refdes,
                    describe_gpios(&gpios)
                ),
            );
        }
    }
}

/// Compatible strings of a part (empty when the chip file has no device tree section).
fn compatibles<'p>(part: &Part<'p>) -> &'p [String] {
    part.chip
        .devicetree
        .as_ref()
        .map_or(&[], |dt| dt.compatible.as_slice())
}

fn is_disabled(node: &Node) -> bool {
    node.string("status").is_some_and(|s| s == "disabled")
}

fn describe_gpios(gpios: &BTreeSet<u32>) -> String {
    if gpios.is_empty() {
        return "no host GPIO".to_owned();
    }
    let list: Vec<String> = gpios.iter().map(u32::to_string).collect();
    format!("GPIO {}", list.join(", "))
}
