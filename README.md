# weft-boardchecker

Checks that a PCB and its Linux device tree overlay describe the same hardware. Part of
[weft](https://theweft.org).

```text
weft-boardchecker --config boardcheck.toml --board board.ipc2581.xml --overlay hat.dtbo
```

It reads the board from a tool-neutral export and the overlay as compiled by `dtc`, then reports:

- devices on the board that the overlay does not describe, and overlay nodes with no device behind them;
- I2C addresses derived from the actual strap wiring (directly or through one resistor), and address collisions;
- SPI chip selects, including `cs-gpios` overrides;
- swapped SDA/SCL or MOSI/MISO;
- wrong driver (`compatible`) for the part at that address;
- `*-gpios` and `interrupts` properties that name a different GPIO than the pin is wired to;
- parts on a host bus that the parts library does not know (warning).

Exit status: 0 clean, 1 findings, 2 unusable input. `--verbose` also lists every passed check.

## Inputs

| Input | Format | Notes |
|---|---|---|
| Board | IPC-2581 rev B/C (preferred) | One file with nets and part numbers. Exported by KiCad 8+, Altium, Cadence Allegro/OrCAD, Siemens Xpedition. Map your part number field in the export dialog (KiCad: "Manufacturer P/N"). |
| Board (fallback) | IPC-D-356A + CSV BOM | For tools without IPC-2581. The BOM needs a designator column and a part number column (auto-detected, or set in `[bom]`). |
| Overlay | compiled `.dtbo` | Build with `dtc -@` so fragment targets and phandle references stay resolvable. `dtc` does the source parsing and preprocessing. |
| Host profile | TOML | Built in: `raspberry-pi-40pin`. Maps connector pins to GPIOs and buses. |
| Parts library | TOML | Built in: TI ADS1115 (VSSOP-10), Microchip MCP4728, 24C32 EEPROMs. Add more with `--parts` or `parts = [...]`. |

## Configuration

```toml
host = "raspberry-pi-40pin"     # or a path to a host profile
connector = "J1"                # designator of the host header on this board
parts = ["parts.toml"]          # extra parts, relative to this file; they take precedence

[mpn]                           # part numbers the export lacks
U7 = "ADS1115IDGSR"

[[expect]]                      # bus devices the overlay intentionally omits
refdes = "U3"
reason = "HAT ID EEPROM, read by the Pi firmware"

ignore = ["U12"]                # designators to skip entirely
```

## Adding a part

Copy an entry from [data/parts.toml](data/parts.toml) and fill it from the datasheet. Pinouts
depend on the package, so make the `mpn` prefix specific enough to select one package.

```toml
[[part]]
name = "TI ADS1115, VSSOP-10"
mpn = ["ADS1115IDGS"]
compatible = ["ti,ads1115"]
bus = "i2c"
pins = { addr = "1", alert = "2", gnd = "3", vdd = "8", sda = "9", scl = "10" }
address = { kind = "strap", pin = "addr", levels = { gnd = 0x48, vdd = 0x49, sda = 0x4a, scl = 0x4b } }
gpios = { interrupts = "alert" }
```

Address rules: `fixed` (one address), `strap` (one pin, levels `gnd`, `vdd` or another pin role)
and `bits` (one pin per address bit). SPI parts need pin roles `sck`, `cs` and, where present,
`mosi` (device input) and `miso` (device output).

## Limits

- Connectivity is followed through at most one resistor. Muxes, level shifters and buffers end the
  trace; devices behind them are reported as not checked.
- Pin names are not in the exports, so the parts library maps pin roles to pin numbers per package.
- The test fixtures are synthetic and shaped after KiCad's IPC-2581 exporter source; validate
  against a real export when the first board exists.

## Development

`nix develop` (or direnv) provides the pinned toolchain, `dtc` and the CI tools. `just ci` runs
everything CI runs. After editing `tests/fixtures/*.dts`, run `just fixtures`.

## Licence

MIT. See [LICENSE](LICENSE).
