// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeMap;
use std::path::Path;

#[derive(Debug, Clone)]
pub(super) struct TypeDefRecord {
    pub token: u32,
    pub namespace: String,
    pub name: String,
    pub entity_kind: String,
    pub enclosing_type: Option<String>,
}

#[derive(Clone, Copy)]
struct Section {
    virtual_address: u32,
    virtual_size: u32,
    raw_offset: u32,
    raw_size: u32,
}

struct RawTypeDef {
    flags: u32,
    namespace: String,
    name: String,
    extends: u32,
}

impl RawTypeName for RawTypeDef {
    fn full_name(&self) -> String {
        qualify(&self.namespace, &self.name)
    }
}

pub(super) fn read_typedefs(path: &Path) -> Result<Vec<TypeDefRecord>, String> {
    let bytes = std::fs::read(path)
        .map_err(|error| format!("Failed to read {}: {error}", path.display()))?;
    let pe = read_u32(&bytes, 0x3c)? as usize;
    if bytes.get(pe..pe + 4) != Some(b"PE\0\0") {
        return Err("Windows.Win32.winmd has no PE signature".into());
    }
    let section_count = read_u16(&bytes, pe + 6)? as usize;
    let optional_size = read_u16(&bytes, pe + 20)? as usize;
    let optional = pe + 24;
    let magic = read_u16(&bytes, optional)?;
    let directories = optional
        + match magic {
            0x10b => 96,
            0x20b => 112,
            _ => return Err(format!("Unsupported PE optional-header magic {magic:#x}")),
        };
    let cli_rva = read_u32(&bytes, directories + 14 * 8)?;
    let section_headers = optional + optional_size;
    let mut sections = Vec::with_capacity(section_count);
    for index in 0..section_count {
        let offset = section_headers + index * 40;
        sections.push(Section {
            virtual_size: read_u32(&bytes, offset + 8)?,
            virtual_address: read_u32(&bytes, offset + 12)?,
            raw_size: read_u32(&bytes, offset + 16)?,
            raw_offset: read_u32(&bytes, offset + 20)?,
        });
    }
    let cli = rva_offset(cli_rva, &sections)?;
    let metadata_rva = read_u32(&bytes, cli + 8)?;
    let metadata = rva_offset(metadata_rva, &sections)?;
    if bytes.get(metadata..metadata + 4) != Some(b"BSJB") {
        return Err("CLI metadata root has no BSJB signature".into());
    }
    let version_length = read_u32(&bytes, metadata + 12)? as usize;
    let mut cursor = align4(metadata + 16 + version_length)?;
    cursor += 2; // flags
    let stream_count = read_u16(&bytes, cursor)? as usize;
    cursor += 2;
    let mut tables = None;
    let mut strings = None;
    for _ in 0..stream_count {
        let offset = read_u32(&bytes, cursor)? as usize;
        let size = read_u32(&bytes, cursor + 4)? as usize;
        let name_start = cursor + 8;
        let name_end = bytes[name_start..]
            .iter()
            .position(|byte| *byte == 0)
            .map(|length| name_start + length)
            .ok_or_else(|| "Unterminated CLI stream name".to_string())?;
        let name = std::str::from_utf8(&bytes[name_start..name_end])
            .map_err(|error| format!("Invalid CLI stream name: {error}"))?;
        match name {
            "#~" | "#-" => tables = Some((metadata + offset, size)),
            "#Strings" => strings = Some((metadata + offset, size)),
            _ => {}
        }
        cursor = align4(name_end + 1)?;
    }
    let (tables, _) = tables.ok_or_else(|| "CLI metadata has no tables stream".to_string())?;
    let (strings_offset, strings_size) =
        strings.ok_or_else(|| "CLI metadata has no #Strings stream".to_string())?;
    let strings = bytes
        .get(strings_offset..strings_offset + strings_size)
        .ok_or_else(|| "#Strings stream is out of bounds".to_string())?;

    let heap_sizes = *bytes
        .get(tables + 6)
        .ok_or_else(|| "Tables header is truncated".to_string())?;
    let valid = read_u64(&bytes, tables + 8)?;
    let mut rows = [0u32; 64];
    let mut table_cursor = tables + 24;
    for (table, row_count) in rows.iter_mut().enumerate() {
        if valid & (1u64 << table) != 0 {
            *row_count = read_u32(&bytes, table_cursor)?;
            table_cursor += 4;
        }
    }
    let sizes = IndexSizes::new(rows, heap_sizes);
    let mut offsets = [0usize; 64];
    for table in 0..64 {
        offsets[table] = table_cursor;
        table_cursor = table_cursor
            .checked_add(
                table_row_size(table, &sizes)?
                    .checked_mul(rows[table] as usize)
                    .ok_or_else(|| "CLI table size overflow".to_string())?,
            )
            .ok_or_else(|| "CLI table offset overflow".to_string())?;
    }

    let mut type_refs = Vec::with_capacity(rows[1] as usize);
    let type_ref_size = table_row_size(1, &sizes)?;
    for row in 0..rows[1] as usize {
        let mut offset = offsets[1] + row * type_ref_size + sizes.resolution_scope();
        let name = read_index(&bytes, &mut offset, sizes.string)?;
        let namespace = read_index(&bytes, &mut offset, sizes.string)?;
        type_refs.push((
            read_string(strings, namespace)?.to_string(),
            read_string(strings, name)?.to_string(),
        ));
    }

    let mut raw = Vec::with_capacity(rows[2] as usize);
    let type_def_size = table_row_size(2, &sizes)?;
    for row in 0..rows[2] as usize {
        let mut offset = offsets[2] + row * type_def_size;
        let flags = read_u32(&bytes, offset)?;
        offset += 4;
        let name = read_index(&bytes, &mut offset, sizes.string)?;
        let namespace = read_index(&bytes, &mut offset, sizes.string)?;
        let extends = read_index(&bytes, &mut offset, sizes.type_def_or_ref())?;
        raw.push(RawTypeDef {
            flags,
            namespace: read_string(strings, namespace)?.to_string(),
            name: read_string(strings, name)?.to_string(),
            extends,
        });
    }

    let mut nested = BTreeMap::new();
    let nested_size = table_row_size(41, &sizes)?;
    for row in 0..rows[41] as usize {
        let mut offset = offsets[41] + row * nested_size;
        let inner = read_index(&bytes, &mut offset, sizes.table(2))?;
        let outer = read_index(&bytes, &mut offset, sizes.table(2))?;
        nested.insert(inner, outer);
    }

    let full_name = |row: u32| -> String {
        fn build(row: u32, raw: &[RawTypeDef], nested: &BTreeMap<u32, u32>) -> String {
            let value = &raw[row as usize - 1];
            if let Some(outer) = nested.get(&row) {
                format!("{}+{}", build(*outer, raw, nested), value.name)
            } else if value.namespace.is_empty() {
                value.name.clone()
            } else {
                format!("{}.{}", value.namespace, value.name)
            }
        }
        build(row, &raw, &nested)
    };

    let mut result = Vec::with_capacity(raw.len().saturating_sub(1));
    for (index, value) in raw.iter().enumerate() {
        let row = index as u32 + 1;
        if value.name == "<Module>" {
            continue;
        }
        let base = decode_type_def_or_ref(value.extends, &raw, &type_refs);
        let entity_kind = if value.flags & 0x20 != 0 {
            "interface"
        } else {
            match base.as_deref() {
                Some("System.Enum") => "enum",
                Some("System.ValueType") => "struct",
                Some("System.MulticastDelegate") => "delegate",
                Some("System.Attribute") => "attribute",
                _ => "class",
            }
        };
        result.push(TypeDefRecord {
            token: 0x0200_0000 | row,
            namespace: value.namespace.clone(),
            name: value.name.clone(),
            entity_kind: entity_kind.into(),
            enclosing_type: nested.get(&row).map(|outer| full_name(*outer)),
        });
    }
    Ok(result)
}

fn decode_type_def_or_ref(
    value: u32,
    defs: &[impl RawTypeName],
    refs: &[(String, String)],
) -> Option<String> {
    let tag = value & 3;
    let row = value >> 2;
    if row == 0 {
        return None;
    }
    match tag {
        0 => defs.get(row as usize - 1).map(RawTypeName::full_name),
        1 => refs
            .get(row as usize - 1)
            .map(|(namespace, name)| qualify(namespace, name)),
        _ => None,
    }
}

trait RawTypeName {
    fn full_name(&self) -> String;
}

impl RawTypeName for (String, String) {
    fn full_name(&self) -> String {
        qualify(&self.0, &self.1)
    }
}

fn qualify(namespace: &str, name: &str) -> String {
    if namespace.is_empty() {
        name.to_string()
    } else {
        format!("{namespace}.{name}")
    }
}

#[derive(Clone, Copy)]
struct IndexSizes {
    rows: [u32; 64],
    string: usize,
    guid: usize,
    blob: usize,
}

impl IndexSizes {
    fn new(rows: [u32; 64], heaps: u8) -> Self {
        Self {
            rows,
            string: if heaps & 1 != 0 { 4 } else { 2 },
            guid: if heaps & 2 != 0 { 4 } else { 2 },
            blob: if heaps & 4 != 0 { 4 } else { 2 },
        }
    }

    fn table(self, table: usize) -> usize {
        if self.rows[table] < 0x1_0000 { 2 } else { 4 }
    }

    fn coded(self, tables: &[usize], tag_bits: u32) -> usize {
        let maximum = tables
            .iter()
            .map(|table| self.rows[*table])
            .max()
            .unwrap_or(0);
        if maximum < (1u32 << (16 - tag_bits)) {
            2
        } else {
            4
        }
    }

    fn type_def_or_ref(self) -> usize {
        self.coded(&[2, 1, 27], 2)
    }

    fn resolution_scope(self) -> usize {
        self.coded(&[0, 26, 35, 1], 2)
    }
}

fn table_row_size(table: usize, s: &IndexSizes) -> Result<usize, String> {
    let size = match table {
        0 => 2 + s.string + s.guid * 3,
        1 => s.resolution_scope() + s.string * 2,
        2 => 4 + s.string * 2 + s.type_def_or_ref() + s.table(4) + s.table(6),
        3 => s.table(4),
        4 => 2 + s.string + s.blob,
        5 => s.table(6),
        6 => 8 + s.string + s.blob + s.table(8),
        7 => s.table(8),
        8 => 4 + s.string,
        9 => s.table(2) + s.type_def_or_ref(),
        10 => s.coded(&[2, 1, 26, 6, 27], 3) + s.string + s.blob,
        11 => 2 + s.coded(&[4, 8, 23], 2) + s.blob,
        12 => {
            s.coded(
                &[
                    6, 4, 1, 2, 8, 9, 10, 0, 14, 23, 20, 17, 26, 27, 32, 35, 38, 39, 40,
                ],
                5,
            ) + s.coded(&[6, 10], 3)
                + s.blob
        }
        13 => s.coded(&[4, 8], 1) + s.blob,
        14 => 2 + s.coded(&[2, 6, 32], 2) + s.blob,
        15 => 6 + s.table(2),
        16 => 4 + s.table(4),
        17 => s.blob,
        18 => s.table(2) + s.table(20),
        19 => s.table(20),
        20 => 2 + s.string + s.type_def_or_ref(),
        21 => s.table(2) + s.table(23),
        22 => s.table(23),
        23 => 2 + s.string + s.blob,
        24 => 2 + s.table(6) + s.coded(&[20, 23], 1),
        25 => s.table(2) + s.coded(&[6, 10], 1) * 2,
        26 => s.string,
        27 => s.blob,
        28 => 2 + s.coded(&[4, 6], 1) + s.string + s.table(26),
        29 => 4 + s.table(4),
        30 => 8,
        31 => 4,
        32 => 16 + s.blob + s.string * 2,
        33 => 4,
        34 => 12,
        35 => 12 + s.blob * 2 + s.string * 2,
        36 => 4 + s.table(35),
        37 => 12 + s.table(35),
        38 => 4 + s.string + s.blob,
        39 => 8 + s.string * 2 + s.coded(&[38, 35, 39], 2),
        40 => 8 + s.string + s.coded(&[38, 35, 39], 2),
        41 => s.table(2) * 2,
        42 => 4 + s.coded(&[2, 6], 1) + s.string,
        43 => s.coded(&[6, 10], 1) + s.blob,
        44 => s.table(42) + s.type_def_or_ref(),
        _ if s.rows[table] == 0 => 0,
        _ => return Err(format!("Unsupported populated CLI table {table}")),
    };
    Ok(size)
}

fn read_string(heap: &[u8], index: u32) -> Result<&str, String> {
    let start = index as usize;
    let length = heap
        .get(start..)
        .ok_or_else(|| "#Strings index is out of bounds".to_string())?
        .iter()
        .position(|byte| *byte == 0)
        .ok_or_else(|| "Unterminated #Strings value".to_string())?;
    std::str::from_utf8(&heap[start..start + length])
        .map_err(|error| format!("Invalid #Strings UTF-8: {error}"))
}

fn read_index(bytes: &[u8], offset: &mut usize, size: usize) -> Result<u32, String> {
    let value = match size {
        2 => read_u16(bytes, *offset)? as u32,
        4 => read_u32(bytes, *offset)?,
        _ => return Err(format!("Invalid metadata index size {size}")),
    };
    *offset += size;
    Ok(value)
}

fn rva_offset(rva: u32, sections: &[Section]) -> Result<usize, String> {
    sections
        .iter()
        .find(|section| {
            rva >= section.virtual_address
                && rva < section.virtual_address + section.virtual_size.max(section.raw_size)
        })
        .map(|section| (section.raw_offset + rva - section.virtual_address) as usize)
        .ok_or_else(|| format!("RVA {rva:#x} is outside PE sections"))
}

fn align4(value: usize) -> Result<usize, String> {
    value
        .checked_add(3)
        .map(|value| value & !3)
        .ok_or_else(|| "Metadata alignment overflow".to_string())
}

fn read_u16(bytes: &[u8], offset: usize) -> Result<u16, String> {
    bytes
        .get(offset..offset + 2)
        .and_then(|value| value.try_into().ok())
        .map(u16::from_le_bytes)
        .ok_or_else(|| format!("Read outside file at {offset:#x}"))
}

fn read_u32(bytes: &[u8], offset: usize) -> Result<u32, String> {
    bytes
        .get(offset..offset + 4)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_le_bytes)
        .ok_or_else(|| format!("Read outside file at {offset:#x}"))
}

fn read_u64(bytes: &[u8], offset: usize) -> Result<u64, String> {
    bytes
        .get(offset..offset + 8)
        .and_then(|value| value.try_into().ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| format!("Read outside file at {offset:#x}"))
}
