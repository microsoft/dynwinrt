// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

#[cfg(test)]
mod tests {
    #[test]
    fn list_property_value_statics_methods() {
        use windows_metadata::*;
        let index = reader::Index::read(
            r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
        )
        .unwrap();
        // IPropertyValueStatics is the exclusive interface of PropertyValue
        let def = index.expect("Windows.Foundation", "IPropertyValueStatics");
        for (i, method) in def.methods().enumerate() {
            println!("  vtable[{}+6] = {} ({:?})", i, method.name(), method.signature(&[]).return_type);
        }

        println!("\nIPropertyValue:");
        let def2 = index.expect("Windows.Foundation", "IPropertyValue");
        for (i, method) in def2.methods().enumerate() {
            println!("  vtable[{}+6] = {} ({:?})", i, method.name(), method.signature(&[]).return_type);
        }
    }


    #[test]
    fn test_winmd_read_uri() {
        use windows_metadata::*;
        let index = reader::Index::read(
            r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
        )
        .unwrap();
        let def = index.expect("Windows.Foundation", "Uri");
        // list all methods and print their signatures
        for method in def.methods() {
            let params: Vec<String> = method
                .params()
                .enumerate()
                .map(|(_i, p)| format!("{}: {:?}", p.name(), method.signature(&[]).types))
                .collect();
            println!(
                "fn {:?} {}({}) -> {:?}",
                method.flags(),
                method.name(),
                params.join(", "),
                method.signature(&[]).return_type
            );
        }
    }

    /// Query: How many WinRT structs have String fields?
    /// Result: Only 4 structs (7 fields), all rare (XAML, Storage, Store).
    /// Conclusion: HString field support is low priority.
    #[test]
    fn find_structs_with_string_fields() {
        use windows_metadata::*;
        let index = reader::Index::read(
            r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd",
        )
        .unwrap();

        let mut count = 0;
        for def in index.all() {
            if let Some(extends) = def.extends() {
                if extends.namespace() == "System" && extends.name() == "ValueType" {
                    for field in def.fields() {
                        if field.ty() == Type::String {
                            println!(
                                "{}.{} -> field '{}' is String",
                                def.namespace(),
                                def.name(),
                                field.name()
                            );
                            count += 1;
                        }
                    }
                }
            }
        }
        println!("\nTotal struct fields that are String: {}", count);
    }

    #[test]
    fn find_structs_with_string_fields_all_winmd() {
        use windows_metadata::*;

        let winmd_dir = r"C:\Program Files\Microsoft Office\root\vfs\ProgramFilesCommonX64\Microsoft Shared\Office16\AI";
        let mut paths: Vec<String> = vec![
            r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd".into(),
        ];
        if let Ok(entries) = std::fs::read_dir(winmd_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("winmd") {
                    paths.push(path.to_string_lossy().into_owned());
                }
            }
        }

        println!("Scanning {} winmd files...\n", paths.len());
        let mut total = 0;
        for path in &paths {
            let index = match reader::Index::read(path) {
                Some(i) => i,
                None => continue,
            };
            for def in index.all() {
                if let Some(extends) = def.extends() {
                    if extends.namespace() == "System" && extends.name() == "ValueType" {
                        let fields: Vec<_> = def.fields().collect();
                        let field_types: Vec<String> = fields.iter()
                            .map(|f| format!("{}: {:?}", f.name(), f.ty()))
                            .collect();
                        let has_non_primitive = fields.iter().any(|f| {
                            matches!(f.ty(), Type::String | Type::Name(..))
                        });
                        if has_non_primitive {
                            println!(
                                "{}.{} {{ {} }}",
                                def.namespace(),
                                def.name(),
                                field_types.join(", ")
                            );
                            total += 1;
                        }
                    }
                }
            }
        }
        println!("\nTotal structs with non-primitive fields: {}", total);
    }

    /// Scan all methods in Windows.winmd + WinAppSDK winmds for array parameter types.
    #[test]
    fn find_array_param_element_types() {
        use std::collections::HashMap;
        use windows_metadata::*;

        let winmd_dir = r"C:\Program Files\Microsoft Office\root\vfs\ProgramFilesCommonX64\Microsoft Shared\Office16\AI";
        let mut paths: Vec<String> = vec![
            r"C:\Program Files (x86)\Windows Kits\10\UnionMetadata\10.0.26100.0\Windows.winmd".into(),
        ];
        if let Ok(entries) = std::fs::read_dir(winmd_dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().and_then(|e| e.to_str()) == Some("winmd") {
                    paths.push(path.to_string_lossy().into_owned());
                }
            }
        }

        let mut counts: HashMap<String, usize> = HashMap::new();
        let mut examples: HashMap<String, Vec<String>> = HashMap::new();

        for path in &paths {
            let index = match reader::Index::read(path) {
                Some(i) => i,
                None => continue,
            };
            for def in index.all() {
                // Skip generic types (signature(&[]) panics for them)
                if def.generic_params().next().is_some() {
                    continue;
                }
                for method in def.methods() {
                    let sig = method.signature(&[]);
                    // Check all parameter types + return type for arrays
                    for ty in &sig.types {
                        let elem_desc = match ty {
                            Type::Array(inner) | Type::ArrayRef(inner) => {
                                format!("{:?}", inner)
                            }
                            _ => continue,
                        };
                        *counts.entry(elem_desc.clone()).or_insert(0) += 1;
                        let example_list = examples.entry(elem_desc).or_insert_with(Vec::new);
                        if example_list.len() < 2 {
                            example_list.push(format!(
                                "{}.{}.{}",
                                def.namespace(),
                                def.name(),
                                method.name()
                            ));
                        }
                    }
                }
            }
        }

        println!("Array element types found in method signatures:\n");
        let mut sorted: Vec<_> = counts.iter().collect();
        sorted.sort_by(|a, b| b.1.cmp(a.1));
        for (ty, count) in &sorted {
            let exs = examples.get(*ty).unwrap();
            println!("  {:>4}x  {}  (e.g. {})", count, ty, exs.join(", "));
        }
        println!("\nTotal distinct array element types: {}", sorted.len());
    }

    #[test]
    fn verify_typed_event_handler_iid() {
        use crate::metadata_table::*;
        use windows_core::{GUID, Interface};

        let table = MetadataTable::new();
        let piid = GUID::from_u128(0x9de1c534_6ae1_11e0_84e1_18a905bcc53f);
        let imembufref_iid = GUID::from_u128(0xfbc4dd29_245b_11e4_af98_689423260cf8);

        let g = table.generic(piid, 2);
        let p = table.parameterized(&g, &[table.interface(imembufref_iid), table.object()]);

        let expected = windows::Foundation::TypedEventHandler::<
            windows::Foundation::IMemoryBufferReference,
            windows_core::IInspectable,
        >::IID;
        assert_eq!(p.iid().unwrap(), expected);
    }
}
