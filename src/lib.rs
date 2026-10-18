// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! Checks a PCB netlist against a Linux device tree overlay.
//!
//! The board comes from a tool-neutral export (IPC-2581, or IPC-D-356A plus a CSV BOM). The overlay
//! is the compiled `.dtbo`. A host profile maps connector pins to GPIOs and buses, and the chip
//! data from `weft-parts` maps part numbers to bus pins and address straps. See [`check::check`].
#![doc(html_logo_url = "https://theweft.org/brand/repos/weft-boardchecker.svg")]
#![doc(html_favicon_url = "https://theweft.org/favicon.svg")]

pub mod board;
pub mod bom;
pub mod check;
pub mod config;
pub mod dtb;
pub mod host;
pub mod ipc2581;
pub mod ipc356;
pub mod overlay;
