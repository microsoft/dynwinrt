// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Reference evidence for reverse WinRT contracts, before the lossy Type projection.

use windows_metadata::{AsRow, ParamAttributes, reader};

pub(super) fn implementation_diagnostics(method: &reader::MethodDef<'_>) -> Vec<String> {
    // windows-metadata 0.59 exposes the MethodDef signature through AsRow, but
    // read_type_signature drops scalar BYREF. Column 4 is Signature (II.22.26).
    let mut blob = method.blob(4);
    let evidence = SignatureReader::new(&blob).read();
    // Blob's debug Drop requires consumption even when evidence parsing fails.
    while !blob.is_empty() {
        blob.read_u8();
    }
    let parameters = match evidence {
        Ok(parameters) => parameters,
        Err(reason) => {
            return vec![format!(
                "unsupported raw ECMA-335 signature evidence: {reason}"
            )];
        }
    };
    let definitions = method
        .params()
        .filter(|parameter| parameter.sequence() > 0)
        .collect::<Vec<_>>();
    parameters
        .into_iter()
        .enumerate()
        .filter_map(|(position, byref)| {
            if !byref {
                return None;
            }
            let sequence = position + 1;
            let definition = definitions.get(position);
            if definition.is_some_and(|parameter| {
                usize::from(parameter.sequence()) == sequence
                    && parameter.flags().contains(ParamAttributes::Out)
            }) {
                return None;
            }
            let name = definition.map_or("", |parameter| parameter.name());
            Some(format!(
                "parameter {name} ({sequence}): input BYREF is not a WinRT implementation contract; an Out contract is required"
            ))
        })
        .collect()
}

pub(super) fn validate_implementation_field(field: &reader::Field<'_>) -> Result<(), String> {
    // Field.Signature is column 2 (II.22.15); Field::ty uses the same lossy reader.
    let mut blob = field.blob(2);
    let evidence = SignatureReader::new(&blob).read_field();
    while !blob.is_empty() {
        blob.read_u8();
    }
    evidence
        .map_err(|reason| format!("unsupported raw ECMA-335 field signature evidence: {reason}"))
}

/// Walk ECMA-335 II.23.2 signatures structurally. Tokens, compressed counts,
/// modifiers and nested types are operands, not candidate BYREF markers.
/// Native type support and Out/Fill/Receive semantics remain in meta.rs.
struct SignatureReader<'a> {
    bytes: &'a [u8],
    position: usize,
}

impl<'a> SignatureReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, position: 0 }
    }

    fn read(mut self) -> Result<Vec<bool>, String> {
        let parameters = self.method(0)?;
        if self.position != self.bytes.len() {
            return Err(self.error("trailing signature bytes"));
        }
        Ok(parameters)
    }

    fn read_field(mut self) -> Result<(), String> {
        if self.byte()? != 0x06 {
            return Err(self.error("not a FIELD signature"));
        }
        self.modifiers()?;
        if self.take(0x10) {
            return Err(self.error("BYREF field has no WinRT value layout"));
        }
        self.typ(0)?;
        if self.position != self.bytes.len() {
            return Err(self.error("trailing field signature bytes"));
        }
        Ok(())
    }

    fn method(&mut self, depth: usize) -> Result<Vec<bool>, String> {
        self.check_depth(depth)?;
        let flags = self.byte()?;
        if !matches!(flags & 0x0f, 0..=5 | 9 | 11) {
            return Err(self.error("not a method calling convention"));
        }
        if flags & 0x10 != 0 {
            self.compressed()?; // GenericParamCount precedes ParamCount.
        }
        let count = self.count()?;
        if self.parameter(true, depth + 1)? {
            return Err(self.error("BYREF return is not a WinRT logical return contract"));
        }
        let mut parameters = Vec::with_capacity(count);
        let mut sentinel = false;
        for _ in 0..count {
            if self.take(0x41) {
                if sentinel || flags & 0x0f != 5 {
                    return Err(self.error("unexpected vararg sentinel"));
                }
                sentinel = true;
            }
            parameters.push(self.parameter(false, depth + 1)?);
        }
        Ok(parameters)
    }

    fn parameter(&mut self, is_return: bool, depth: usize) -> Result<bool, String> {
        self.modifiers()?;
        let byref = self.take(0x10);
        if (!byref && self.take(0x16)) || (is_return && !byref && self.take(0x01)) {
            return Ok(byref);
        }
        self.typ(depth)?;
        Ok(byref)
    }

    fn typ(&mut self, depth: usize) -> Result<(), String> {
        self.check_depth(depth)?;
        match self.byte()? {
            0x02..=0x0e | 0x18 | 0x19 | 0x1c => {}
            0x0f => {
                self.modifiers()?;
                if !self.take(0x01) {
                    self.typ(depth + 1)?;
                }
            }
            0x11 | 0x12 => self.type_token()?,
            0x13 | 0x1e => {
                self.compressed()?;
            }
            0x14 => {
                self.typ(depth + 1)?;
                let rank = self.compressed()?;
                let sizes = self.count()?;
                if rank == 0 || sizes > rank as usize {
                    return Err(self.error("invalid array rank or size count"));
                }
                for _ in 0..sizes {
                    self.compressed()?;
                }
                let bounds = self.count()?;
                if bounds > rank as usize {
                    return Err(self.error("invalid array lower-bound count"));
                }
                for _ in 0..bounds {
                    // Signed compressed lower bounds use the same byte widths.
                    self.compressed()?;
                }
            }
            0x15 => {
                if !matches!(self.byte()?, 0x11 | 0x12) {
                    return Err(self.error("generic instance requires CLASS or VALUETYPE"));
                }
                self.type_token()?;
                let count = self.count()?;
                if count == 0 {
                    return Err(self.error("generic instance has no type arguments"));
                }
                for _ in 0..count {
                    self.typ(depth + 1)?;
                }
            }
            0x1b => {
                self.method(depth + 1)?;
            }
            0x1d => {
                self.modifiers()?;
                self.typ(depth + 1)?;
            }
            0x10 => return Err(self.error("nested BYREF is not a WinRT value type")),
            element => {
                return Err(self.error(&format!("unsupported type element 0x{element:02x}")));
            }
        }
        Ok(())
    }

    fn modifiers(&mut self) -> Result<(), String> {
        while self.take(0x1f) || self.take(0x20) {
            self.type_token()?;
        }
        Ok(())
    }

    fn type_token(&mut self) -> Result<(), String> {
        let token = self.compressed()?;
        if token >> 2 == 0 || token & 3 == 3 {
            return Err(self.error("invalid TypeDefOrRefEncoded token"));
        }
        Ok(())
    }

    fn count(&mut self) -> Result<usize, String> {
        let count = self.compressed()? as usize;
        if count > self.bytes.len() - self.position {
            return Err(self.error("count exceeds remaining signature bytes"));
        }
        Ok(count)
    }

    fn compressed(&mut self) -> Result<u32, String> {
        let first = self.byte()?;
        match first {
            0x00..=0x7f => Ok(u32::from(first)),
            0x80..=0xbf => Ok((u32::from(first & 0x3f) << 8) | u32::from(self.byte()?)),
            0xc0..=0xdf => Ok((u32::from(first & 0x1f) << 24)
                | (u32::from(self.byte()?) << 16)
                | (u32::from(self.byte()?) << 8)
                | u32::from(self.byte()?)),
            _ => Err(self.error("invalid compressed integer")),
        }
    }

    fn byte(&mut self) -> Result<u8, String> {
        let value = self
            .bytes
            .get(self.position)
            .copied()
            .ok_or_else(|| self.error("truncated signature"))?;
        self.position += 1;
        Ok(value)
    }

    fn take(&mut self, value: u8) -> bool {
        if self.bytes.get(self.position) == Some(&value) {
            self.position += 1;
            true
        } else {
            false
        }
    }

    fn check_depth(&self, depth: usize) -> Result<(), String> {
        if depth > 64 {
            Err(self.error("signature nesting exceeds the evidence reader limit"))
        } else {
            Ok(())
        }
    }

    fn error(&self, reason: &str) -> String {
        format!("byte {}: {reason}", self.position)
    }
}

#[cfg(test)]
mod tests {
    use super::SignatureReader;

    #[test]
    fn compressed_counts_and_type_operands_are_not_qualifiers() {
        let mut many_parameters = vec![0x20, 0x80, 0x80, 0x01];
        many_parameters.extend([0x08; 128]);
        assert_eq!(
            SignatureReader::new(&many_parameters).read().unwrap(),
            vec![false; 128]
        );
        for typ in [
            vec![0x11, 0x10],                   // TypeDef token
            vec![0x13, 0x10],                   // VAR number
            vec![0x1e, 0x10],                   // MVAR number
            vec![0x14, 0x08, 1, 1, 0x10, 1, 0], // ARRAY size
            vec![0x14, 0x08, 1, 0, 1, 0x80, 1], // signed lower bound -8192
            vec![0x15, 0x12, 0x10, 1, 0x15, 0x12, 0x10, 1, 0x08],
            vec![0x0f, 0x20, 0x10, 0x01], // PTR custom modifier and VOID
            vec![0x1b, 0, 1, 1, 0x10, 0x08], // FNPTR's parameter, not the outer one
        ] {
            let mut signature = vec![0x20, 2, 1];
            signature.extend(typ);
            signature.extend([0x10, 0x08]);
            assert_eq!(
                SignatureReader::new(&signature).read().unwrap(),
                [false, true]
            );
        }
        assert_eq!(
            SignatureReader::new(&[0x30, 0x10, 1, 1, 0x13, 0x10])
                .read()
                .unwrap(),
            [false]
        );
    }

    #[test]
    fn malformed_evidence_fails_closed() {
        for signature in [
            &[0x20, 1, 1, 0x10][..],
            &[0x20, 1, 1, 0x10, 0x10, 0x08],
            &[0x20, 1, 1, 0x20, 0x80],
            &[0x20, 1, 1, 0x15, 0x12, 0x10, 1, 0x10, 0x08],
            &[0x20, 1, 1, 0x1d],
            &[0x20, 1, 1, 0x08, 0x08],
            &[0x20, 0xff, 1, 0x08],
        ] {
            assert!(
                SignatureReader::new(signature).read().is_err(),
                "{signature:?}"
            );
        }
        let mut nested = vec![0x20, 1, 1];
        nested.extend([0x1d; 65]);
        nested.push(0x08);
        assert!(
            SignatureReader::new(&nested)
                .read()
                .unwrap_err()
                .contains("nesting")
        );
    }

    #[test]
    fn field_evidence_preserves_reference_qualifiers_after_modifiers() {
        assert!(SignatureReader::new(&[0x06, 0x08]).read_field().is_ok());
        for modifier in [0x1f, 0x20] {
            assert!(
                SignatureReader::new(&[0x06, modifier, 0x10, 0x08])
                    .read_field()
                    .is_ok()
            );
            assert!(
                SignatureReader::new(&[0x06, modifier, 0x10, 0x10, 0x08])
                    .read_field()
                    .unwrap_err()
                    .contains("BYREF field")
            );
        }
        for signature in [&[0x20, 0x08][..], &[0x06], &[0x06, 0x08, 0x08]] {
            assert!(SignatureReader::new(signature).read_field().is_err());
        }
    }
}
