// SPDX-FileCopyrightText: 2026 The weft authors
// SPDX-License-Identifier: MIT

//! Minimal flattened device tree (DTB/DTBO) reader: builds a tree of nodes and raw properties.
//! Only reading is needed; source parsing, includes and preprocessing stay with `dtc`.

use anyhow::{Context, Result, bail};

const MAGIC: u32 = 0xd00d_feed;
const BEGIN_NODE: u32 = 1;
const END_NODE: u32 = 2;
const PROP: u32 = 3;
const NOP: u32 = 4;
const END: u32 = 9;

/// A device tree node.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Node {
    /// Full node name including unit address, such as `adc@48`. Empty for the root.
    pub name: String,
    /// Properties in blob order.
    pub props: Vec<(String, Vec<u8>)>,
    /// Child nodes in blob order.
    pub children: Vec<Node>,
}

impl Node {
    /// Raw value of a property.
    pub fn prop(&self, name: &str) -> Option<&[u8]> {
        self.props
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_slice())
    }

    /// A property as a list of NUL-terminated strings.
    pub fn strings(&self, name: &str) -> Vec<String> {
        self.prop(name).map(strings).unwrap_or_default()
    }

    /// A property as a single string (the first of a string list).
    pub fn string(&self, name: &str) -> Option<String> {
        self.strings(name).into_iter().next()
    }

    /// A property as big-endian 32-bit cells.
    pub fn cells(&self, name: &str) -> Vec<u32> {
        self.prop(name).map(cells).unwrap_or_default()
    }

    /// Child by full name.
    pub fn child(&self, name: &str) -> Option<&Node> {
        self.children.iter().find(|c| c.name == name)
    }
}

/// Splits a property value into NUL-terminated strings.
pub fn strings(value: &[u8]) -> Vec<String> {
    value
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect()
}

/// Splits a property value into big-endian 32-bit cells (a trailing partial cell is ignored).
pub fn cells(value: &[u8]) -> Vec<u32> {
    value
        .as_chunks::<4>()
        .0
        .iter()
        .map(|c| u32::from_be_bytes(*c))
        .collect()
}

/// Parses a DTB or DTBO into its root node.
///
/// # Errors
/// Fails on a bad magic number, out-of-range offsets or a malformed structure block.
pub fn parse(blob: &[u8]) -> Result<Node> {
    let header = |index: usize| -> Result<usize> {
        let start = index * 4;
        let bytes = blob
            .get(start..start + 4)
            .context("DTB: truncated header")?;
        Ok(be32(bytes)? as usize)
    };
    if header(0)? != MAGIC as usize {
        bail!("not a device tree blob (bad magic); pass the compiled .dtbo, not the .dts");
    }
    let total = header(1)?;
    let off_struct = header(2)?;
    let off_strings = header(3)?;
    let size_strings = header(8)?;
    let size_struct = header(9)?;
    if total > blob.len() {
        bail!("DTB: truncated ({} of {total} bytes)", blob.len());
    }
    let structure = blob
        .get(off_struct..off_struct.saturating_add(size_struct))
        .context("DTB: structure block out of range")?;
    let strings_block = blob
        .get(off_strings..off_strings.saturating_add(size_strings))
        .context("DTB: strings block out of range")?;

    let mut reader = Reader {
        data: structure,
        pos: 0,
    };
    // Stack of open nodes; the root is pushed by its BEGIN_NODE token.
    let mut stack: Vec<Node> = Vec::new();
    loop {
        match reader.u32()? {
            BEGIN_NODE => {
                let name = reader.cstr()?;
                stack.push(Node {
                    name,
                    ..Node::default()
                });
            }
            END_NODE => {
                let node = stack.pop().context("DTB: unbalanced END_NODE")?;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(node),
                    None => {
                        // Root closed; only END (or NOPs) may follow.
                        loop {
                            match reader.u32()? {
                                NOP => {}
                                END => return Ok(node),
                                token => bail!("DTB: unexpected token {token} after root"),
                            }
                        }
                    }
                }
            }
            PROP => {
                let len = reader.u32()? as usize;
                let name_off = reader.u32()? as usize;
                let value = reader.bytes(len)?.to_vec();
                let name_bytes = strings_block
                    .get(name_off..)
                    .context("DTB: bad name offset")?;
                let end = name_bytes
                    .iter()
                    .position(|b| *b == 0)
                    .context("DTB: unterminated name")?;
                let name = String::from_utf8_lossy(name_bytes.get(..end).unwrap_or_default());
                stack
                    .last_mut()
                    .context("DTB: property outside a node")?
                    .props
                    .push((name.into_owned(), value));
            }
            NOP => {}
            END => bail!("DTB: END before the root node closed"),
            token => bail!("DTB: unknown token {token}"),
        }
    }
}

fn be32(bytes: &[u8]) -> Result<u32> {
    let array: [u8; 4] = bytes.try_into().context("DTB: expected 4 bytes")?;
    Ok(u32::from_be_bytes(array))
}

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn u32(&mut self) -> Result<u32> {
        be32(self.bytes(4)?)
    }

    /// Reads `len` bytes and skips padding to the next 4-byte boundary.
    fn bytes(&mut self, len: usize) -> Result<&'a [u8]> {
        let end = self.pos.checked_add(len).context("DTB: length overflow")?;
        let out = self
            .data
            .get(self.pos..end)
            .context("DTB: truncated structure block")?;
        self.pos = end.next_multiple_of(4);
        Ok(out)
    }

    fn cstr(&mut self) -> Result<String> {
        let rest = self
            .data
            .get(self.pos..)
            .context("DTB: truncated structure block")?;
        let len = rest
            .iter()
            .position(|b| *b == 0)
            .context("DTB: unterminated node name")?;
        let bytes = self.bytes(len + 1)?;
        Ok(String::from_utf8_lossy(bytes.get(..len).unwrap_or_default()).into_owned())
    }
}
