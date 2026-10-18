// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! End-to-end checks on the synthetic demo board (tests/fixtures).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    missing_docs
)]

use std::collections::{BTreeMap, BTreeSet};

use weft_boardcheck::board::Board;
use weft_boardcheck::check::{self, Finding, Input, Severity};
use weft_boardcheck::host::Host;
use weft_boardcheck::overlay::Overlay;
use weft_boardcheck::parts::Library;
use weft_boardcheck::{bom, ipc356, ipc2581};

const DEMO_XML: &str = include_str!("fixtures/demo.ipc2581.xml");
const DEMO_356: &str = include_str!("fixtures/demo.ipc356");
const DEMO_BOM: &str = include_str!("fixtures/demo.bom.csv");
const TEST_PARTS: &str = include_str!("fixtures/test-parts.toml");
const GOOD: &[u8] = include_bytes!("fixtures/good.dtbo");
const WRONG: &[u8] = include_bytes!("fixtures/wrong.dtbo");
const CS: &[u8] = include_bytes!("fixtures/cs.dtbo");

fn demo() -> Board {
    ipc2581::parse(DEMO_XML).unwrap()
}

fn run(board: &Board, overlay: &[u8]) -> Vec<Finding> {
    let mut parts = Library::builtin().unwrap();
    parts.add_toml(TEST_PARTS, "test parts").unwrap();
    let host = Host::builtin("raspberry-pi-40pin").unwrap();
    let overlay = Overlay::parse(overlay).unwrap();
    let expect = BTreeMap::from([("U3".to_owned(), "HAT ID EEPROM".to_owned())]);
    let ignore = BTreeSet::new();
    check::check(Input {
        board,
        overlay: &overlay,
        host: &host,
        parts: &parts,
        connector: "J1",
        expect: &expect,
        ignore: &ignore,
    })
}

fn with(findings: &[Finding], severity: Severity) -> Vec<String> {
    findings
        .iter()
        .filter(|f| f.severity == severity)
        .map(|f| format!("{}: {}", f.subject, f.message))
        .collect()
}

fn assert_one(findings: &[Finding], severity: Severity, needle: &str) {
    let hits = with(findings, severity);
    assert!(
        hits.iter().any(|m| m.contains(needle)),
        "no {severity:?} containing {needle:?}; got {findings:#?}"
    );
}

#[test]
fn ipc2581_reads_nets_and_part_numbers() {
    let board = demo();
    assert_eq!(board.components.len(), 8);
    assert_eq!(board.net_of("U1", "9"), Some("I2C1_SDA"));
    // From the approved vendor list.
    assert_eq!(board.components["U1"].mpn.as_deref(), Some("ADS1115IDGSR"));
    // From a BOM characteristic named MPN.
    assert_eq!(board.components["U4"].mpn.as_deref(), Some("TESTSPI-1"));
    assert_eq!(board.components["R1"].mpn, None);
}

#[test]
fn ipc356_with_bom_equals_ipc2581() {
    let mut board = ipc356::parse(DEMO_356).unwrap();
    bom::apply(&mut board, DEMO_BOM, &bom::Columns::default()).unwrap();
    // Long net names come through NNAME aliases.
    assert_eq!(board.net_of("U1", "4"), Some("ADC_INPUT_CHANNEL_0"));
    let mut expected = demo();
    for component in expected.components.values_mut() {
        component.mpn = component.mpn.take();
    }
    assert_eq!(board.components, expected.components);
    let sorted = |b: &Board| {
        b.nets
            .iter()
            .map(|(n, p)| (n.clone(), p.iter().cloned().collect::<BTreeSet<_>>()))
            .collect::<BTreeMap<_, _>>()
    };
    assert_eq!(sorted(&board), sorted(&expected));
}

#[test]
fn matching_overlay_passes() {
    let findings = run(&demo(), GOOD);
    assert!(with(&findings, Severity::Error).is_empty(), "{findings:#?}");
    assert!(
        with(&findings, Severity::Warning).is_empty(),
        "{findings:#?}"
    );
    assert_one(
        &findings,
        Severity::Ok,
        "U1 (ADS1115IDGSR) at i2c1 address 0x48 matches",
    );
    assert_one(
        &findings,
        Severity::Ok,
        "U1 interrupts: GPIO 23 reaches pin 2",
    );
    assert_one(
        &findings,
        Severity::Ok,
        "U4 reset-gpios: GPIO 22 reaches pin 5",
    );
    // A0 reaches ground through R1.
    assert_one(
        &findings,
        Severity::Ok,
        "U3 (24LC32AT-I/SN) is at i2c0 address 0x50",
    );
}

#[test]
fn wrong_overlay_reports_each_mistake() {
    let findings = run(&demo(), WRONG);
    assert_eq!(with(&findings, Severity::Error).len(), 5, "{findings:#?}");
    assert_one(
        &findings,
        Severity::Error,
        "U1: U1 (ADS1115IDGSR) is at i2c1 address 0x48, but the overlay has no node there. The overlay has /fragment@0/__overlay__/adc@49",
    );
    assert_one(
        &findings,
        Severity::Error,
        "adc@49: The overlay declares \"ti,ads1115\" at i2c1 address 0x49",
    );
    assert_one(
        &findings,
        Severity::Error,
        "compatible \"microchip,mcp4725\" (expected \"microchip,mcp4728\")",
    );
    assert_one(
        &findings,
        Severity::Error,
        "reset-gpios uses GPIO 24, but U4 pin 5 (reset) is wired to GPIO 22",
    );
    assert_one(&findings, Severity::Error, "ghost@30");
}

#[test]
fn address_strap_to_vdd_moves_the_adc() {
    let mut board = demo();
    board.rewire("U1", "1", "+3V3");
    let findings = run(&board, GOOD);
    assert_one(
        &findings,
        Severity::Error,
        "U1 (ADS1115IDGSR) is at i2c1 address 0x49",
    );
}

#[test]
fn address_strap_to_sda() {
    let mut board = demo();
    board.rewire("U1", "1", "I2C1_SDA");
    assert_one(&run(&board, GOOD), Severity::Error, "i2c1 address 0x4a");
}

#[test]
fn floating_address_pin_is_an_error() {
    let mut board = demo();
    board.rewire("U1", "1", "ADDR_FLOAT");
    assert_one(
        &run(&board, GOOD),
        Severity::Error,
        "address pin (addr) is on net ADDR_FLOAT",
    );
}

#[test]
fn address_collision() {
    let mut board = demo();
    for (pin, net) in [
        ("1", "GND"),
        ("3", "GND"),
        ("8", "+3V3"),
        ("9", "I2C1_SDA"),
        ("10", "I2C1_SCL"),
    ] {
        board.connect(net, "U9", pin);
    }
    board.set_mpn("U9", "ADS1115IDGST");
    assert_one(
        &run(&board, GOOD),
        Severity::Error,
        "U1 and U9 share i2c1 address 0x48",
    );
}

#[test]
fn swapped_i2c_lines() {
    let mut board = demo();
    board.rewire("U2", "2", "I2C1_SDA");
    board.rewire("U2", "3", "I2C1_SCL");
    assert_one(
        &run(&board, GOOD),
        Severity::Error,
        "U2 has SDA and SCL swapped on i2c1",
    );
}

#[test]
fn swapped_spi_data_lines() {
    let mut board = demo();
    board.rewire("U4", "3", "SPI0_MISO");
    board.rewire("U4", "4", "SPI0_MOSI");
    assert_one(
        &run(&board, GOOD),
        Severity::Error,
        "U4 has MOSI and MISO swapped on spi0",
    );
}

#[test]
fn chip_select_on_a_plain_gpio_needs_cs_gpios() {
    let mut board = demo();
    // Header pin 29 is GPIO 5.
    board.connect("SENSOR_CS", "J1", "29");
    board.rewire("U4", "1", "SENSOR_CS");
    assert_one(
        &run(&board, GOOD),
        Severity::Error,
        "chip select reaches GPIO 5, which is not a chip select of spi0",
    );
    // This overlay only describes spi0, so only U4 must be clean.
    let findings = run(&board, CS);
    assert!(
        with(&findings, Severity::Error)
            .iter()
            .all(|m| !m.starts_with("U4")),
        "{findings:#?}"
    );
    assert_one(
        &findings,
        Severity::Ok,
        "U4 (TESTSPI-1) at spi0 chip select 1 matches",
    );
}

#[test]
fn unknown_part_on_a_bus_is_a_warning() {
    let mut board = demo();
    for (pin, net) in [
        ("1", "I2C1_SDA"),
        ("2", "I2C1_SCL"),
        ("4", "GND"),
        ("8", "+3V3"),
    ] {
        board.connect(net, "U7", pin);
    }
    board.set_mpn("U7", "LM75BDP");
    assert_one(
        &run(&board, GOOD),
        Severity::Warning,
        "U7 (LM75BDP) pin 1 is on i2c1 SDA",
    );
}

#[test]
fn missing_connector_is_reported() {
    let mut board = demo();
    board.components.remove("J1");
    assert_one(
        &run(&board, GOOD),
        Severity::Error,
        "host connector J1 is not on the board",
    );
}

#[test]
fn builtin_data_is_valid() {
    let parts = Library::builtin().unwrap();
    assert!(parts.lookup("ADS1115IDGSR").is_some());
    assert!(parts.lookup("ads1115idgst").is_some());
    assert!(
        parts.lookup("ADS1115IRUGR").is_none(),
        "other package, other pinout"
    );
    for name in Host::builtin_names() {
        Host::builtin(name).unwrap();
    }
}

#[test]
fn rejects_dts_source_and_garbage() {
    assert!(Overlay::parse(b"/dts-v1/;").is_err());
    assert!(Overlay::parse(&GOOD[..40]).is_err());
    assert!(ipc2581::parse("<html/>").is_err());
    assert!(ipc356::parse("C nothing\n999\n").is_err());
}
