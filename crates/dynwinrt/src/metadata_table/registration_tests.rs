// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::sync::Barrier;

const ISTRINGABLE: GUID = GUID::from_u128(0x96369f54_8eb6_48f0_abce_c1b211e627c3);
const ICLOSABLE: GUID = GUID::from_u128(0x30d5a829_7fa4_4026_83bb_d75bae4ea99e);

fn register_stringable(table: &Arc<MetadataTable>, name: &str) -> TypeHandle {
    table.register_interface(name, ISTRINGABLE).add_method(
        "ToString",
        MethodSignature::new(table).add_out(table.hstring()),
    )
}

fn register_closable(table: &Arc<MetadataTable>, name: &str) -> TypeHandle {
    table
        .register_interface(name, ICLOSABLE)
        .add_method("Close", MethodSignature::new(table))
}

#[test]
fn same_name_different_iids_keep_independent_methods_in_both_orders() {
    for reverse in [false, true] {
        let table = MetadataTable::new();
        let (stringable, closable) = if reverse {
            let closable = register_closable(&table, "SharedInterface");
            (register_stringable(&table, "SharedInterface"), closable)
        } else {
            let stringable = register_stringable(&table, "SharedInterface");
            (stringable, register_closable(&table, "SharedInterface"))
        };
        assert_eq!(stringable, table.interface(ISTRINGABLE));
        assert_eq!(closable, table.interface(ICLOSABLE));
        assert!(stringable.method_by_name("Close").is_none());
        assert!(closable.method_by_name("ToString").is_none());
        assert_ne!(
            stringable.method(6).unwrap().index,
            closable.method(6).unwrap().index
        );
        assert!(stringable.method(7).is_none());
        assert!(closable.method(7).is_none());
    }
}

#[test]
fn same_iid_aliases_and_repeated_methods_preserve_published_method() {
    let table = MetadataTable::new();
    let first = register_stringable(&table, "IStringable");
    let original = first.method(6).unwrap();
    let original_ptr = table.method_ptr(original.index);
    for name in [
        "IStringable",
        "Windows.Foundation.IStringable",
        "AnotherPackage.IStringable",
    ] {
        let repeated = register_stringable(&table, name);
        assert_eq!(repeated, first);
        assert_eq!(
            repeated.method_by_name("ToString").unwrap().index,
            original.index
        );
        assert_eq!(
            table.method_ptr(repeated.method(6).unwrap().index),
            original_ptr
        );
        assert!(repeated.method(7).is_none());
    }
    assert_eq!(table.interface_methods.read().unwrap().len(), 1);
}

#[test]
fn interface_names_do_not_alias_or_replace_other_named_types() {
    for interface_first in [false, true] {
        for category in 0..3 {
            let table = MetadataTable::new();
            let name = "SharedName";
            let register_named = || match category {
                0 => table.struct_type(name, &[table.i32_type()]),
                1 => table.enum_type(name, vec![("Value".into(), 1)]),
                2 => table.runtime_class(name.into(), &table.interface(ISTRINGABLE)),
                _ => unreachable!(),
            };
            let (interface, named) = if interface_first {
                (table.register_interface(name, ICLOSABLE), register_named())
            } else {
                let named = register_named();
                (table.register_interface(name, ICLOSABLE), named)
            };
            assert_eq!(interface, table.interface(ICLOSABLE));
            assert!(matches!(
                (category, named.kind()),
                (0, TypeKind::Struct(_)) | (1, TypeKind::Enum(_)) | (2, TypeKind::RuntimeClass(_))
            ));
            assert_eq!(table.get_named_type(name), Some(named.kind()));
            assert_eq!(register_named(), named);
            assert_eq!(register_closable(&table, name), interface);
            assert_eq!(table.get_named_type(name), Some(named.kind()));
        }
    }
}

#[test]
fn concurrent_registration_deduplicates_only_by_iid() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<MetadataTable>();
    assert_send_sync::<TypeHandle>();
    assert_send_sync::<MethodHandle>();

    let table = MetadataTable::new();
    let barrier = Arc::new(Barrier::new(16));
    let threads: Vec<_> = (0..16)
        .map(|index| {
            let table = Arc::clone(&table);
            let barrier = Arc::clone(&barrier);
            std::thread::spawn(move || {
                barrier.wait();
                let name = if index % 4 < 2 {
                    "SharedInterface"
                } else {
                    "Alias"
                };
                let interface = if index % 2 == 0 {
                    register_stringable(&table, name)
                } else {
                    register_closable(&table, name)
                };
                let expected = if index % 2 == 0 {
                    ISTRINGABLE
                } else {
                    ICLOSABLE
                };
                assert_eq!(interface.iid(), Some(expected));
                assert!(interface.method(7).is_none());
                (expected, interface.method(6).unwrap().index)
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|thread| thread.join()).collect();
    for result in results {
        let (iid, index) = result.unwrap();
        assert_eq!(table.interface(iid).method(6).unwrap().index, index);
    }
    let methods = table.interface_methods.read().unwrap();
    assert_eq!(methods.len(), 2);
    assert!(
        methods
            .values()
            .all(|entry| entry.method_indices.len() == 1)
    );
}
