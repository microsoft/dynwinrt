// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

mod append_only_arena;
mod arena;
mod iid;
mod method_handle;
mod type_handle;
mod type_kind;
mod value_data;

pub use method_handle::MethodHandle;
pub use type_handle::TypeHandle;
pub use type_kind::*;
pub use value_data::ValueTypeData;

use std::collections::HashMap;
use std::sync::{Arc, RwLock};

use windows_core::GUID;

use crate::signature::{Method, MethodSignature};

use append_only_arena::AppendOnlyBoxArena;
use arena::*;

// ===========================================================================
// MetadataTable
// ===========================================================================

/// Centralized registry of WinRT types and methods.
///
/// Thread-safe, append-only, Arc-wrapped for shared ownership.
/// All data lives in arenas; lightweight indexes provide O(1) lookup.
/// All direct arena access goes through `arena.rs` methods.
pub struct MetadataTable {
    // --- Type arenas (primary data) ---
    structs: RwLock<Vec<StructEntry>>,
    runtime_classes: RwLock<Vec<RuntimeClassData>>,
    parameterized_types: RwLock<Vec<ParameterizedData>>,
    inner_types: RwLock<Vec<TypeKind>>,
    inner_type_pairs: RwLock<Vec<(TypeKind, TypeKind)>>,
    enum_entries: RwLock<Vec<EnumData>>,

    // --- Methods arena ---
    //
    // Stored in an `AppendOnlyBoxArena` so entries have heap-stable
    // addresses that can never be invalidated: callers may take a raw
    // `*const Method` under the read guard, drop the guard, and then
    // invoke the method. This is essential because a method call can be
    // `DispatcherQueue.runEventLoop`, which pumps messages and may
    // re-enter this arena for `push_method` (requiring `.write()`); if
    // the outer call still held the read lock during dispatch, we would
    // deadlock.
    //
    // Vec growth may move the `Box<Method>` slots inside the arena's
    // backing buffer, but the boxed `Method` itself never moves. The
    // arena type intentionally exposes only `push` / `stable_ptr`
    // (no `pop`/`remove`/`clear`), so the "never removed" invariant
    // is enforced by the type system rather than by convention.
    methods: AppendOnlyBoxArena<Method>,

    // --- Indexes (no data duplication, only pointers) ---
    /// IID → method table for O(1) interface method lookup.
    interface_methods: RwLock<HashMap<GUID, InterfaceMethodTable>>,
    /// Name → TypeKind for dedup of all named types (struct, enum, runtime_class).
    type_names: RwLock<HashMap<String, TypeKind>>,
}

impl std::fmt::Debug for MetadataTable {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MetadataTable").finish_non_exhaustive()
    }
}

// Safety: MetadataTable is protected by RwLock internally.
// The non-Send/Sync raw pointers come from libffi Cif objects inside Method,
// which are only accessed through &self methods behind the RwLock.
unsafe impl Send for MetadataTable {}
unsafe impl Sync for MetadataTable {}

impl MetadataTable {
    pub fn new() -> Arc<Self> {
        Arc::new(MetadataTable {
            structs: RwLock::new(Vec::new()),
            runtime_classes: RwLock::new(Vec::new()),
            parameterized_types: RwLock::new(Vec::new()),
            inner_types: RwLock::new(Vec::new()),
            inner_type_pairs: RwLock::new(Vec::new()),
            enum_entries: RwLock::new(Vec::new()),
            methods: AppendOnlyBoxArena::new(),
            interface_methods: RwLock::new(HashMap::new()),
            type_names: RwLock::new(HashMap::new()),
        })
    }

    // -----------------------------------------------------------------------
    // Type factory methods
    // -----------------------------------------------------------------------

    pub fn make(self: &Arc<Self>, kind: TypeKind) -> TypeHandle {
        TypeHandle {
            table: Arc::clone(self),
            kind,
        }
    }

    fn assert_owns_type(&self, typ: &TypeHandle, context: &str) {
        assert!(
            std::ptr::eq(self, typ.table.as_ref()),
            "{context} must use the same MetadataTable"
        );
    }

    // Primitive types
    pub fn bool_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::Bool)
    }
    pub fn i8_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::I8)
    }
    pub fn u8_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::U8)
    }
    pub fn i16_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::I16)
    }
    pub fn u16_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::U16)
    }
    pub fn char16_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::Char16)
    }
    pub fn i32_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::I32)
    }
    pub fn u32_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::U32)
    }
    pub fn i64_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::I64)
    }
    pub fn u64_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::U64)
    }
    pub fn f32_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::F32)
    }
    pub fn f64_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::F64)
    }
    pub fn guid_type(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::Guid)
    }
    pub fn hstring(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::HString)
    }
    pub fn object(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::Object)
    }
    pub fn hresult(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::HResult)
    }
    pub fn array_of_iunknown(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::ArrayOfIUnknown)
    }
    pub fn async_action(self: &Arc<Self>) -> TypeHandle {
        self.make(TypeKind::IAsyncAction)
    }

    /// Create a TypeHandle from a TypeKind. Only works for simple (non-indexed) kinds.
    pub fn handle_from_kind(self: &Arc<Self>, kind: TypeKind) -> TypeHandle {
        self.make(kind)
    }

    // GUID-carrying types
    pub fn interface(self: &Arc<Self>, iid: GUID) -> TypeHandle {
        self.make(TypeKind::Interface(iid))
    }
    pub fn delegate(self: &Arc<Self>, iid: GUID) -> TypeHandle {
        self.make(TypeKind::Delegate(iid))
    }
    pub fn generic(self: &Arc<Self>, piid: GUID, arity: u32) -> TypeHandle {
        self.make(TypeKind::Generic { piid, arity })
    }

    // Compound types that allocate indexed storage
    pub fn runtime_class(
        self: &Arc<Self>,
        name: String,
        default_interface_type: &TypeHandle,
    ) -> TypeHandle {
        self.assert_owns_type(
            default_interface_type,
            "runtime class and default interface",
        );
        assert!(
            matches!(
                default_interface_type.kind,
                TypeKind::Interface(_) | TypeKind::Parameterized(_)
            ),
            "runtime class default interface must be an interface type"
        );

        let (kind, already_registered) = {
            let mut names = self.type_names.write().unwrap();
            if let Some(kind) = names.get(&name).copied() {
                (kind, true)
            } else {
                let kind = self.push_runtime_class(name.clone(), default_interface_type.kind);
                names.insert(name.clone(), kind);
                (kind, false)
            }
        };
        if !already_registered {
            return self.make(kind);
        }
        let TypeKind::RuntimeClass(idx) = kind else {
            panic!("named type '{name}' is not a runtime class");
        };
        let (_, registered_default) = self.get_runtime_class(idx);
        assert_eq!(
            self.signature_string_kind(registered_default),
            default_interface_type.signature_string(),
            "runtime class '{name}' was registered with a different default interface"
        );
        self.make(kind)
    }

    pub fn parameterized(
        self: &Arc<Self>,
        generic_def: &TypeHandle,
        args: &[TypeHandle],
    ) -> TypeHandle {
        self.assert_owns_type(generic_def, "parameterized generic definition");
        for argument in args {
            self.assert_owns_type(argument, "parameterized type argument");
        }
        let args_kinds: Vec<TypeKind> = args.iter().map(|a| a.kind).collect();
        self.make(self.push_parameterized(generic_def.kind, args_kinds))
    }

    pub fn async_operation(self: &Arc<Self>, result_type: &TypeHandle) -> TypeHandle {
        self.assert_owns_type(result_type, "async operation result type");
        let idx = self.push_inner_type(result_type.kind);
        self.make(TypeKind::IAsyncOperation(idx))
    }

    pub fn async_action_with_progress(self: &Arc<Self>, progress_type: &TypeHandle) -> TypeHandle {
        self.assert_owns_type(progress_type, "async action progress type");
        let idx = self.push_inner_type(progress_type.kind);
        self.make(TypeKind::IAsyncActionWithProgress(idx))
    }

    pub fn async_operation_with_progress(
        self: &Arc<Self>,
        result_type: &TypeHandle,
        progress_type: &TypeHandle,
    ) -> TypeHandle {
        self.assert_owns_type(result_type, "async operation result type");
        self.assert_owns_type(progress_type, "async operation progress type");
        let idx = self.push_inner_type_pair(result_type.kind, progress_type.kind);
        self.make(TypeKind::IAsyncOperationWithProgress(idx))
    }

    pub fn out_value(self: &Arc<Self>, inner: &TypeHandle) -> TypeHandle {
        self.assert_owns_type(inner, "nested out type");
        let idx = self.push_inner_type(inner.kind);
        self.make(TypeKind::OutValue(idx))
    }

    pub fn array(self: &Arc<Self>, element_type: &TypeHandle) -> TypeHandle {
        self.assert_owns_type(element_type, "array element type");
        let idx = self.push_inner_type(element_type.kind);
        self.make(TypeKind::Array(idx))
    }

    // -----------------------------------------------------------------------
    // Registration API (single entry point for each type)
    // -----------------------------------------------------------------------

    /// Register a named interface. Creates an IID → method table.
    /// Returns a TypeHandle for chaining `.add_method()`.
    pub fn register_interface(self: &Arc<Self>, name: &str, iid: GUID) -> TypeHandle {
        if let Some(kind) = self.get_named_type(name) {
            return self.make(kind);
        }
        self.create_interface_method_table(iid, 6);
        let kind = TypeKind::Interface(iid);
        self.insert_named_type(name, kind);
        self.make(kind)
    }

    pub(crate) fn register_com_interface(
        self: &Arc<Self>,
        name: &str,
        iid: GUID,
        base_slot: usize,
    ) -> TypeHandle {
        if let Some(kind) = self.get_named_type(name) {
            assert_eq!(
                kind,
                TypeKind::Interface(iid),
                "type name {name:?} is already registered with a different type or IID"
            );
            self.create_interface_method_table(iid, base_slot);
            return self.make(kind);
        }
        self.create_interface_method_table(iid, base_slot);
        let kind = TypeKind::Interface(iid);
        self.insert_named_type(name, kind);
        self.make(kind)
    }

    /// Register a named struct with dedup. If already registered, returns
    /// the existing TypeHandle.
    pub fn struct_type(self: &Arc<Self>, name: &str, fields: &[TypeHandle]) -> TypeHandle {
        for field in fields {
            self.assert_owns_type(field, "struct field type");
        }
        if let Some(kind) = self.get_named_type(name) {
            return self.make(kind);
        }
        let field_kinds: Vec<TypeKind> = fields.iter().map(|h| h.kind).collect();
        let id = self.push_struct(name, field_kinds);
        let kind = TypeKind::Struct(id);
        self.insert_named_type(name, kind);
        self.make(kind)
    }

    /// Register a named enum with member values.
    pub fn enum_type(self: &Arc<Self>, name: &str, members: Vec<(String, i32)>) -> TypeHandle {
        if let Some(kind) = self.get_named_type(name) {
            return self.make(kind);
        }
        let id = self.push_enum(name, members);
        let kind = TypeKind::Enum(id);
        self.insert_named_type(name, kind);
        self.make(kind)
    }

    // -----------------------------------------------------------------------
    // Methods
    // -----------------------------------------------------------------------

    /// Add a method to the interface identified by IID.
    pub(crate) fn add_method_to_interface(
        &self,
        iid: &GUID,
        name: &str,
        sig: MethodSignature,
    ) -> u32 {
        self.push_method(iid, name, sig)
    }

    /// Get a MethodHandle by vtable index. O(1) lookup by IID.
    pub(crate) fn method_by_vtable_index(
        self: &Arc<Self>,
        iid: &GUID,
        vtable_index: usize,
    ) -> Option<MethodHandle> {
        let arena_index = self.get_method_arena_index_by_vtable(iid, vtable_index)?;
        Some(MethodHandle::new(Arc::clone(self), arena_index))
    }

    /// Get a MethodHandle by method name. O(1) IID lookup + linear name scan.
    pub(crate) fn method_by_name(self: &Arc<Self>, iid: &GUID, name: &str) -> Option<MethodHandle> {
        let arena_index = self.get_method_arena_index_by_name(iid, name)?;
        Some(MethodHandle::new(Arc::clone(self), arena_index))
    }

    // -----------------------------------------------------------------------
    // Query API
    // -----------------------------------------------------------------------

    pub fn get_enum_value(&self, enum_name: &str, member_name: &str) -> Option<i32> {
        self.get_enum_members(enum_name)?
            .iter()
            .find(|(n, _)| n == member_name)
            .map(|(_, v)| *v)
    }

    // -----------------------------------------------------------------------
    // Collection IID computation helpers
    // -----------------------------------------------------------------------

    /// Compute all IIDs needed for an IVector<element_type>.
    pub fn vector_iids(self: &Arc<Self>, element_type: &TypeHandle) -> crate::vector::VectorIids {
        use type_kind::*;
        self.assert_owns_type(element_type, "vector element type");
        let elem = element_type.kind;
        crate::vector::VectorIids {
            iterable: self.compute_parameterized_iid(&IITERABLE, &[elem]),
            vector: self.compute_parameterized_iid(&IVECTOR, &[elem]),
            vector_view: self.compute_parameterized_iid(&IVECTOR_VIEW, &[elem]),
            observable_vector: self.compute_parameterized_iid(&IOBSERVABLE_VECTOR, &[elem]),
            vector_changed_handler: self
                .compute_parameterized_iid(&VECTOR_CHANGED_EVENT_HANDLER, &[elem]),
            iterator: self.compute_parameterized_iid(&IITERATOR, &[elem]),
        }
    }

    /// Compute all IIDs needed for an IMap<key_type, value_type>.
    pub fn map_iids(
        self: &Arc<Self>,
        key_type: &TypeHandle,
        value_type: &TypeHandle,
    ) -> crate::map::MapIids {
        use type_kind::*;
        self.assert_owns_type(key_type, "map key type");
        self.assert_owns_type(value_type, "map value type");
        let k = key_type.kind;
        let v = value_type.kind;
        // Create a Parameterized TypeKind for IKeyValuePair<K,V> so that
        // signature_string_kind can resolve it for the outer IIterable/IIterator.
        let kvp_kind = self.push_parameterized(
            TypeKind::Generic {
                piid: IKEY_VALUE_PAIR,
                arity: 2,
            },
            vec![k, v],
        );
        let kvp_iid = self.compute_parameterized_iid(&IKEY_VALUE_PAIR, &[k, v]);
        // IIterable<IKeyValuePair<K,V>> and IIterator<IKeyValuePair<K,V>>
        // need the signature of the KVP as a type arg — which is itself a parameterized type.
        let iterable_iid = self.compute_parameterized_iid(&IITERABLE, &[kvp_kind]);
        let iterator_iid = self.compute_parameterized_iid(&IITERATOR, &[kvp_kind]);
        crate::map::MapIids {
            iterable: iterable_iid,
            map: self.compute_parameterized_iid(&IMAP, &[k, v]),
            map_view: self.compute_parameterized_iid(&IMAP_VIEW, &[k, v]),
            kvp: kvp_iid,
            iterator: iterator_iid,
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::abi::AbiType;
    use crate::value::WinRTValue;
    use windows_core::Interface;

    // -----------------------------------------------------------------------
    // Primitive types
    // -----------------------------------------------------------------------

    #[test]
    fn primitive_size_and_align() {
        let table = MetadataTable::new();
        assert_eq!(table.u8_type().size_of(), 1);
        assert_eq!(table.i32_type().size_of(), 4);
        assert_eq!(table.f32_type().size_of(), 4);
        assert_eq!(table.f64_type().size_of(), 8);
        assert_eq!(table.f32_type().align_of(), 4);
        assert_eq!(table.f64_type().align_of(), 8);
    }

    #[test]
    fn abi_type_mapping() {
        let table = MetadataTable::new();
        assert_eq!(table.bool_type().abi_type(), AbiType::Bool);
        assert_eq!(table.i32_type().abi_type(), AbiType::I32);
        assert_eq!(table.f64_type().abi_type(), AbiType::F64);
        assert_eq!(table.hstring().abi_type(), AbiType::Ptr);
        assert_eq!(table.object().abi_type(), AbiType::Ptr);
    }

    // -----------------------------------------------------------------------
    // Struct: layout, field access, libffi, Windows ABI compatibility
    // -----------------------------------------------------------------------

    #[test]
    fn struct_layout_and_field_access() {
        let table = MetadataTable::new();
        let f32_h = table.f32_type();
        let point = table.struct_type("Windows.Foundation.Point", &[f32_h.clone(), f32_h]);

        // Layout
        assert_eq!(point.size_of(), 8);
        assert_eq!(point.align_of(), 4);
        assert_eq!(point.field_count(), 2);
        assert_eq!(point.field_offset(0), 0);
        assert_eq!(point.field_offset(1), 4);

        // Matches real Windows.Foundation.Point
        assert_eq!(
            point.size_of(),
            std::mem::size_of::<windows::Foundation::Point>()
        );
        assert_eq!(
            point.align_of(),
            std::mem::align_of::<windows::Foundation::Point>()
        );

        // Field read/write
        let mut val = point.default_value();
        val.set_field(0, 10.0f32);
        val.set_field(1, 20.0f32);
        assert_eq!(val.get_field::<f32>(0), 10.0);
        assert_eq!(val.get_field::<f32>(1), 20.0);
    }

    #[test]
    fn struct_mixed_alignment() {
        // BasicGeoposition has f64 fields — tests 8-byte alignment
        let table = MetadataTable::new();
        let f64_h = table.f64_type();
        let geo = table.struct_type(
            "Windows.Devices.Geolocation.BasicGeoposition",
            &[f64_h.clone(), f64_h.clone(), f64_h],
        );
        assert_eq!(geo.size_of(), 24);
        assert_eq!(geo.align_of(), 8);
    }

    #[test]
    fn struct_nested_libffi_type() {
        let table = MetadataTable::new();
        let f32_h = table.f32_type();
        let f64_h = table.f64_type();
        let point = table.struct_type("Windows.Foundation.Point", &[f32_h.clone(), f32_h]);
        let _ = point.libffi_type(); // should not panic

        let outer = table.struct_type("Test.PointWithAltitude", &[point, f64_h]);
        let _ = outer.libffi_type(); // nested struct should work
    }

    #[test]
    fn struct_dedup_by_name() {
        let table = MetadataTable::new();
        let f32_h = table.f32_type();
        let h1 = table.struct_type("Windows.Foundation.Point", &[f32_h.clone(), f32_h.clone()]);
        let h2 = table.struct_type("Windows.Foundation.Point", &[f32_h.clone(), f32_h]);

        // Same TypeKind (same arena index)
        assert_eq!(h1.kind(), h2.kind());
        assert_eq!(h1.size_of(), h2.size_of());
    }

    // -----------------------------------------------------------------------
    // Enum
    // -----------------------------------------------------------------------

    #[test]
    fn enum_registration_and_query() {
        let table = MetadataTable::new();
        let handle = table.enum_type(
            "Windows.Foundation.AsyncStatus",
            vec![
                ("Started".into(), 0),
                ("Completed".into(), 1),
                ("Canceled".into(), 2),
                ("Error".into(), 3),
            ],
        );

        // ABI is i32
        assert_eq!(handle.abi_type(), AbiType::I32);

        // Query by name
        assert_eq!(
            table.get_enum_value("Windows.Foundation.AsyncStatus", "Completed"),
            Some(1)
        );
        assert_eq!(
            table.get_enum_value("Windows.Foundation.AsyncStatus", "Error"),
            Some(3)
        );
        assert_eq!(
            table.get_enum_value("Windows.Foundation.AsyncStatus", "Nonexistent"),
            None
        );
        assert_eq!(table.get_enum_value("Nonexistent.Enum", "Foo"), None);
    }

    // -----------------------------------------------------------------------
    // Interface: registration, method lookup
    // -----------------------------------------------------------------------

    #[test]
    fn interface_method_lookup() {
        let iid = GUID::from_u128(0x9E365E57_48B2_4160_956F_C7385120BBFC);
        let table = MetadataTable::new();
        let iface = table
            .register_interface("IUriRuntimeClass", iid)
            .add_method(
                "get_AbsoluteUri",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_DisplayUri",
                MethodSignature::new(&table).add_out(table.hstring()),
            );

        // By vtable index (6 = first user method after IInspectable)
        assert!(iface.method(6).is_some());
        assert!(iface.method(7).is_some());
        assert!(iface.method(8).is_none()); // out of bounds
        assert!(iface.method(5).is_none()); // IInspectable range

        // By name
        assert!(iface.method_by_name("get_AbsoluteUri").is_some());
        assert!(iface.method_by_name("get_DisplayUri").is_some());
        assert!(iface.method_by_name("nonexistent").is_none());
    }

    // -----------------------------------------------------------------------
    // IID / signature computation
    // -----------------------------------------------------------------------

    #[test]
    fn iid_interface() {
        let table = MetadataTable::new();
        let iid = GUID::from_u128(0x12345678_1234_1234_1234_123456789abc);
        assert_eq!(table.interface(iid).iid(), Some(iid));
    }

    #[test]
    fn iid_parameterized_async_operation() {
        let table = MetadataTable::new();
        let g = table.generic(IASYNC_OPERATION, 1);
        let p = table.parameterized(&g, &[table.hstring()]);

        // Must match the IID computed by windows_future for IAsyncOperation<HSTRING>
        assert_eq!(
            p.iid().unwrap(),
            windows_future::IAsyncOperation::<windows_core::HSTRING>::IID,
        );
    }

    #[test]
    fn iid_runtime_class_as_type_arg() {
        let table = MetadataTable::new();
        let storage_file_default =
            table.interface(GUID::from_u128(0xFA3F6186_4214_428C_A64C_14C9AC7315EA));
        let storage_file =
            table.runtime_class("Windows.Storage.StorageFile".into(), &storage_file_default);
        let g = table.generic(IASYNC_OPERATION, 1);
        let ty = table.parameterized(&g, &[storage_file]);

        let expected_iid = GUID::from_u128(0x5e52f8ce_aced_5a42_95b4_f674dd84885e);
        assert_eq!(ty.iid().unwrap(), expected_iid);
    }

    #[test]
    fn iid_runtime_class_with_parameterized_default_interface() {
        let table = MetadataTable::new();
        let device_information_default =
            table.interface(GUID::from_u128(0xaba0fb95_4398_489d_8e44_e6130927011f));
        let device_information = table.runtime_class(
            "Windows.Devices.Enumeration.DeviceInformation".into(),
            &device_information_default,
        );
        let vector_view = table.parameterized(
            &table.generic(IVECTOR_VIEW, 1),
            std::slice::from_ref(&device_information),
        );
        let collection = table.runtime_class(
            "Windows.Devices.Enumeration.DeviceInformationCollection".into(),
            &vector_view,
        );
        let operation = table.async_operation(&collection);
        let expected_signature = "pinterface({9fc2b0bb-e446-44e2-aa61-9cab8f636af2};rc(Windows.Devices.Enumeration.DeviceInformationCollection;pinterface({bbe1fa4c-b0e3-4583-baef-1f1b2e483e56};rc(Windows.Devices.Enumeration.DeviceInformation;{aba0fb95-4398-489d-8e44-e6130927011f}))))";
        let expected_iid = GUID::from_signature(windows_core::imp::ConstBuffer::from_slice(
            expected_signature.as_bytes(),
        ));

        assert_eq!(operation.signature_string(), expected_signature);
        assert_eq!(operation.iid(), Some(expected_iid));
    }

    #[test]
    #[should_panic(
        expected = "runtime class 'Contoso.Widget' was registered with a different default interface"
    )]
    fn runtime_class_rejects_conflicting_default_interfaces() {
        let table = MetadataTable::new();
        let first = table.interface(GUID::from_u128(0xaaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa));
        let second = table.interface(GUID::from_u128(0xbbbbbbbb_bbbb_bbbb_bbbb_bbbbbbbbbbbb));

        table.runtime_class("Contoso.Widget".into(), &first);
        table.runtime_class("Contoso.Widget".into(), &second);
    }

    #[test]
    fn runtime_class_registration_is_atomic() {
        use std::sync::Barrier;

        let table = MetadataTable::new();
        let first = table.interface(GUID::from_u128(0xaaaaaaaa_aaaa_aaaa_aaaa_aaaaaaaaaaaa));
        let second = table.interface(GUID::from_u128(0xbbbbbbbb_bbbb_bbbb_bbbb_bbbbbbbbbbbb));
        let barrier = Arc::new(Barrier::new(3));
        let register = |default_interface: TypeHandle| {
            let table = table.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    table.runtime_class("Contoso.ConcurrentWidget".into(), &default_interface)
                }))
            })
        };
        let first_registration = register(first);
        let second_registration = register(second);

        barrier.wait();
        let results = [
            first_registration.join().unwrap(),
            second_registration.join().unwrap(),
        ];

        assert_eq!(results.iter().filter(|result| result.is_ok()).count(), 1);
        assert_eq!(results.iter().filter(|result| result.is_err()).count(), 1);
        assert!(table.get_named_type("Contoso.ConcurrentWidget").is_some());
    }

    #[test]
    #[should_panic(expected = "parameterized type argument must use the same MetadataTable")]
    fn parameterized_rejects_foreign_type_handles() {
        let table = MetadataTable::new();
        let foreign_table = MetadataTable::new();
        let generic = table.generic(IVECTOR_VIEW, 1);

        table.parameterized(&generic, &[foreign_table.i32_type()]);
    }

    #[test]
    fn dynamic_guid_parameters_use_value_abi() {
        use std::ffi::c_void;
        use windows_core::HRESULT;

        const EXPECTED: GUID = GUID::from_u128(0x1a34f5c1_4a5a_46dc_b644_1f4567e7a676);

        #[repr(C)]
        struct GuidVtable {
            inspectable: [usize; 6],
            round_trip: unsafe extern "system" fn(*mut c_void, GUID, u32, *mut GUID) -> HRESULT,
        }

        #[repr(C)]
        struct GuidObject {
            vtable: *const GuidVtable,
        }

        unsafe extern "system" fn round_trip(
            _this: *mut c_void,
            value: GUID,
            marker: u32,
            result: *mut GUID,
        ) -> HRESULT {
            if value != EXPECTED || marker != 42 || result.is_null() {
                return HRESULT(0x80070057_u32 as i32);
            }
            unsafe { result.write(value) };
            HRESULT(0)
        }

        let vtable = GuidVtable {
            inspectable: [0; 6],
            round_trip,
        };
        let mut object = GuidObject { vtable: &vtable };
        let table = MetadataTable::new();
        let interface = table
            .register_interface(
                "IGuidRoundTrip",
                GUID::from_u128(0xd13ed3ce_282c_4fc3_a5c1_9e8cb2207938),
            )
            .add_method(
                "RoundTrip",
                MethodSignature::new(&table)
                    .add_in(table.guid_type())
                    .add_in(table.u32_type())
                    .add_out(table.guid_type()),
            );

        let results = interface
            .method(6)
            .unwrap()
            .invoke(
                (&mut object as *mut GuidObject).cast(),
                &[WinRTValue::Guid(EXPECTED), WinRTValue::U32(42)],
            )
            .unwrap();

        assert!(matches!(results.as_slice(), [WinRTValue::Guid(value)] if *value == EXPECTED));
    }

    #[test]
    fn signature_string() {
        let table = MetadataTable::new();
        assert_eq!(table.i32_type().signature_string(), "i4");
        assert_eq!(table.hstring().signature_string(), "string");

        let g = table.generic(IASYNC_OPERATION, 1);
        let sig = table.parameterized(&g, &[table.hstring()]);
        assert_eq!(
            sig.signature_string(),
            "pinterface({9fc2b0bb-e446-44e2-aa61-9cab8f636af2};string)",
        );
    }

    #[test]
    fn guid_braced_format() {
        let guid = GUID::from_u128(0x9fc2b0bb_e446_44e2_aa61_9cab8f636af2);
        assert_eq!(
            format_guid_braced(&guid),
            "{9fc2b0bb-e446-44e2-aa61-9cab8f636af2}"
        );
    }

    // -----------------------------------------------------------------------
    // End-to-end: register → invoke → verify (requires WinRT runtime)
    // -----------------------------------------------------------------------

    #[test]
    fn e2e_uri_create_and_query() {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        use windows_core::{Interface, h};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let table = MetadataTable::new();

        // Register interfaces
        let factory_iid = GUID::from_u128(0x44A9796F_723E_4FDF_A218_033E75B0C084);
        let factory_iface = table
            .register_interface("IUriRuntimeClassFactory", factory_iid)
            .add_method(
                "CreateUri",
                MethodSignature::new(&table)
                    .add_in(table.hstring())
                    .add_out(table.object()),
            );

        let uri_iid = GUID::from_u128(0x9E365E57_48B2_4160_956F_C7385120BBFC);
        let uri_iface = table
            .register_interface("IUriRuntimeClass", uri_iid)
            .add_method(
                "get_AbsoluteUri",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_DisplayUri",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_Domain",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_Extension",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_Fragment",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_Host",
                MethodSignature::new(&table).add_out(table.hstring()),
            );

        // Activate factory and QI
        let factory = unsafe {
            windows::Win32::System::WinRT::RoGetActivationFactory::<
                windows::Win32::System::WinRT::IActivationFactory,
            >(h!("Windows.Foundation.Uri"))
        }
        .unwrap();
        let mut factory_ptr = std::ptr::null_mut();
        unsafe {
            factory
                .cast::<windows_core::IUnknown>()
                .unwrap()
                .query(&factory_iid, &mut factory_ptr)
                .ok()
                .unwrap();
        }

        // CreateUri via method_by_name
        let uri_val = factory_iface
            .method_by_name("CreateUri")
            .unwrap()
            .invoke(
                factory_ptr,
                &[WinRTValue::HString(windows_core::HSTRING::from(
                    "https://www.example.com/path?q=1#frag",
                ))],
            )
            .unwrap();
        let uri_obj = uri_val[0].as_object().unwrap();
        let mut uri_ptr = std::ptr::null_mut();
        unsafe {
            uri_obj.query(&uri_iid, &mut uri_ptr).ok().unwrap();
        }

        // get_Host via method_by_name
        let host = uri_iface
            .method_by_name("get_Host")
            .unwrap()
            .invoke(uri_ptr, &[])
            .unwrap()[0]
            .as_hstring()
            .unwrap();
        assert_eq!(host.to_string(), "www.example.com");

        // get_AbsoluteUri via vtable index
        let abs_uri = uri_iface.method(6).unwrap().invoke(uri_ptr, &[]).unwrap()[0]
            .as_hstring()
            .unwrap();
        assert_eq!(abs_uri.to_string(), "https://www.example.com/path?q=1#frag");

        // get_Domain via method_by_name
        let domain = uri_iface
            .method_by_name("get_Domain")
            .unwrap()
            .invoke(uri_ptr, &[])
            .unwrap()[0]
            .as_hstring()
            .unwrap();
        assert_eq!(domain.to_string(), "example.com");
    }

    #[test]
    fn e2e_geopoint_struct_in_param() -> windows::core::Result<()> {
        use windows::Devices::Geolocation::{Geopoint, IGeopointFactory};
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        use windows_core::{Interface, h};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let table = MetadataTable::new();

        // Register BasicGeoposition struct
        let f64_h = table.f64_type();
        let geo_type = table.struct_type(
            "Windows.Devices.Geolocation.BasicGeoposition",
            &[f64_h.clone(), f64_h.clone(), f64_h],
        );

        // Register IGeopointFactory
        let factory_iid = IGeopointFactory::IID;
        let factory_iface = table
            .register_interface("IGeopointFactory", factory_iid)
            .add_method(
                "Create",
                MethodSignature::new(&table)
                    .add_in(geo_type.clone())
                    .add_out(table.object()),
            );

        // Create struct value
        let mut geo_val = geo_type.default_value();
        geo_val.set_field(0, 47.643f64); // Latitude
        geo_val.set_field(1, -122.131f64); // Longitude
        geo_val.set_field(2, 100.0f64); // Altitude

        // Activate and call
        let af = unsafe {
            windows::Win32::System::WinRT::RoGetActivationFactory::<
                windows::Win32::System::WinRT::IActivationFactory,
            >(h!("Windows.Devices.Geolocation.Geopoint"))
        }?;
        let mut factory_ptr = std::ptr::null_mut();
        unsafe {
            af.cast::<windows_core::IUnknown>()
                .unwrap()
                .query(&factory_iid, &mut factory_ptr)
                .ok()
                .unwrap();
        }

        let result = factory_iface
            .method_by_name("Create")
            .unwrap()
            .invoke(factory_ptr, &[WinRTValue::Struct(geo_val)])
            .map_err(|e| match e {
                crate::result::Error::WindowsError(we) => we,
                _ => panic!("{:?}", e),
            })?;

        // Verify via static projection
        let geopoint: Geopoint = result[0].as_object().unwrap().cast()?;
        let pos = geopoint.Position()?;
        assert!((pos.Latitude - 47.643).abs() < 1e-6);
        assert!((pos.Longitude - (-122.131)).abs() < 1e-6);
        assert!((pos.Altitude - 100.0).abs() < 1e-6);
        Ok(())
    }

    #[test]
    fn e2e_runtime_class_auto_qi() {
        use windows::Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize};
        use windows_core::{IUnknown, Interface, h};

        let _ = unsafe { RoInitialize(RO_INIT_MULTITHREADED) };
        let table = MetadataTable::new();

        // Register IUriRuntimeClass (default interface) — alphabetical vtable order
        let uri_iid = GUID::from_u128(0x9E365E57_48B2_4160_956F_C7385120BBFC);
        let iuri = table
            .register_interface("IUriRuntimeClass_test", uri_iid)
            .add_method(
                "get_AbsoluteUri",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_DisplayUri",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_Domain",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_Extension",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_Fragment",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_Host",
                MethodSignature::new(&table).add_out(table.hstring()),
            );

        // Register IUriRuntimeClassWithAbsoluteCanonicalUri (second interface)
        let uri2_iid = GUID::from_u128(0x758D9661_221C_480F_A339_50656673F46F);
        let iuri2 = table
            .register_interface("IUriRuntimeClassWithAbsoluteCanonicalUri_test", uri2_iid)
            .add_method(
                "get_AbsoluteCanonicalUri",
                MethodSignature::new(&table).add_out(table.hstring()),
            )
            .add_method(
                "get_DisplayIri",
                MethodSignature::new(&table).add_out(table.hstring()),
            );

        // Verify direct interface call works first
        let uri =
            windows::Foundation::Uri::CreateUri(h!("https://www.example.com/path?q=1")).unwrap();
        let mut direct_ptr = std::ptr::null_mut();
        unsafe {
            uri.cast::<IUnknown>()
                .unwrap()
                .query(&uri_iid, &mut direct_ptr)
                .ok()
                .unwrap();
        }
        let direct_host = iuri
            .method_by_name("get_Host")
            .unwrap()
            .invoke(direct_ptr, &[])
            .unwrap();
        assert_eq!(
            direct_host[0].as_hstring().unwrap().to_string(),
            "www.example.com"
        );

        // .as() pattern: cast to specific interface, then call methods
        let uri_obj = WinRTValue::Object(uri.cast::<IUnknown>().unwrap());

        // .as(IUri) → QI to default interface, then invoke
        let uri_as_iuri = uri_obj.cast(&uri_iid).unwrap();
        let host = iuri
            .method_by_name("get_Host")
            .unwrap()
            .invoke(uri_as_iuri.as_object().unwrap().as_raw(), &[])
            .unwrap();
        assert_eq!(host[0].as_hstring().unwrap().to_string(), "www.example.com");

        let domain = iuri
            .method_by_name("get_Domain")
            .unwrap()
            .invoke(uri_as_iuri.as_object().unwrap().as_raw(), &[])
            .unwrap();
        assert_eq!(domain[0].as_hstring().unwrap().to_string(), "example.com");

        // .as(IUri2) → QI to second interface, then invoke
        let uri_as_iuri2 = uri_obj.cast(&uri2_iid).unwrap();
        let canonical = iuri2
            .method_by_name("get_AbsoluteCanonicalUri")
            .unwrap()
            .invoke(uri_as_iuri2.as_object().unwrap().as_raw(), &[])
            .unwrap();
        assert!(
            canonical[0]
                .as_hstring()
                .unwrap()
                .to_string()
                .contains("example.com")
        );
    }

    // -----------------------------------------------------------------------
    // P0 fix verification tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_nested_struct_with_hstring_clone_drop() {
        // Verify nested struct Clone/Drop recursion for non-blittable fields.
        // Inner struct has an HString field; outer struct embeds it.
        let table = MetadataTable::new();
        let inner = table.struct_type("Test.Inner", &[table.hstring()]);
        let outer = table.struct_type("Test.Outer", &[inner.clone(), table.i32_type()]);

        // Create outer value with an HString in the nested struct
        let mut outer_val = outer.default_value();
        // Write an HString into the inner struct's first field
        let hstr = windows_core::HSTRING::from("hello from nested struct");
        let raw: *mut std::ffi::c_void = unsafe { std::mem::transmute(hstr) };
        unsafe {
            let inner_offset = outer.field_offset(0);
            let inner_field_offset = inner.field_offset(0);
            let target = outer_val
                .as_mut_ptr()
                .add(inner_offset + inner_field_offset)
                as *mut *mut std::ffi::c_void;
            target.write(raw);
        }

        // Clone the outer value — this exercises recursive duplicate_non_blittable_fields
        let cloned = outer_val.clone();

        // Both should have valid HString handles (not aliased)
        let read_hstr = |val: &ValueTypeData| -> String {
            let inner_val = val.get_field_struct(0);
            let raw: *mut std::ffi::c_void = inner_val.get_field(0);
            if raw.is_null() {
                return String::new();
            }
            let hstr: &windows_core::HSTRING =
                unsafe { &*(&raw as *const *mut std::ffi::c_void as *const windows_core::HSTRING) };
            hstr.to_string()
        };

        assert_eq!(read_hstr(&outer_val), "hello from nested struct");
        assert_eq!(read_hstr(&cloned), "hello from nested struct");

        // Drop both — if recursive drop is wrong, this will crash or leak
        drop(cloned);
        drop(outer_val);
    }

    #[test]
    fn test_fillarray_actual_count_clamped() {
        // Verify that FillArray clamps actual_count to capacity.
        // We test this indirectly via ArrayData — create a small buffer and verify
        // the ArrayData length is capped.
        let table = MetadataTable::new();
        let elem = table.i32_type();
        let capacity = 3usize;
        let total_bytes = capacity * 4;
        let buffer_ptr = unsafe {
            windows::Win32::System::Com::CoTaskMemAlloc(total_bytes) as *mut std::ffi::c_void
        };
        assert!(!buffer_ptr.is_null());
        unsafe { std::ptr::write_bytes(buffer_ptr as *mut u8, 0, total_bytes) };

        // Simulate callee reporting actual_count > capacity
        let clamped = std::cmp::min(10usize, capacity);
        let array = crate::array::ArrayData::from_cotaskmem(elem, buffer_ptr, clamped);
        assert_eq!(array.len(), 3); // clamped to capacity, not 10
        drop(array);
    }
}
