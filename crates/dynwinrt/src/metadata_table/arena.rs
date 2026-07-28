// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::alloc::Layout;

use windows_core::GUID;

use super::MetadataTable;
use super::type_kind::TypeKind;
use crate::signature::MethodSignature;
use crate::value::WinRTValue;

// ===========================================================================
// Arena data types
// ===========================================================================

pub(super) struct RuntimeClassData {
    pub(super) name: String,
    pub(super) default_interface: TypeKind,
}

pub(super) struct ParameterizedData {
    pub(super) generic_def: TypeKind,
    pub(super) args: Vec<TypeKind>,
}

pub(super) struct StructEntry {
    /// WinRT full name (e.g. "Windows.Web.Http.HttpProgress") for IID signature computation.
    pub(super) name: String,
    pub(super) field_kinds: Vec<TypeKind>,
    pub(super) field_offsets: Vec<usize>,
    pub(super) layout: Layout,
}

pub(super) struct EnumData {
    pub(super) name: String,
    pub(super) members: Vec<(String, i32)>,
}

pub(super) struct InterfaceMethodTable {
    pub(super) method_names: Vec<String>,
    pub(super) method_indices: Vec<u32>,
    pub(super) base_slot: usize,
}

// ===========================================================================
// Arena write operations
// ===========================================================================

impl MetadataTable {
    // -----------------------------------------------------------------------
    // Type arena writes (return TypeKind, not TypeHandle)
    // -----------------------------------------------------------------------

    pub(super) fn push_runtime_class(&self, name: String, default_interface: TypeKind) -> TypeKind {
        let mut rcs = self.runtime_classes.write().unwrap();
        let idx = rcs.len() as u32;
        rcs.push(RuntimeClassData {
            name,
            default_interface,
        });
        TypeKind::RuntimeClass(idx)
    }

    pub(super) fn push_parameterized(
        &self,
        generic_def: TypeKind,
        args: Vec<TypeKind>,
    ) -> TypeKind {
        let mut pts = self.parameterized_types.write().unwrap();
        let idx = pts.len() as u32;
        pts.push(ParameterizedData { generic_def, args });
        TypeKind::Parameterized(idx)
    }

    pub(super) fn push_inner_type(&self, kind: TypeKind) -> u32 {
        let mut inner = self.inner_types.write().unwrap();
        let idx = inner.len() as u32;
        inner.push(kind);
        idx
    }

    pub(super) fn push_inner_type_pair(&self, a: TypeKind, b: TypeKind) -> u32 {
        let mut pairs = self.inner_type_pairs.write().unwrap();
        let idx = pairs.len() as u32;
        pairs.push((a, b));
        idx
    }

    /// Push a named struct into arena. Returns the arena index.
    /// Does NOT check for duplicates — caller must handle dedup.
    pub(super) fn push_struct(&self, name: &str, field_kinds: Vec<TypeKind>) -> u32 {
        let (field_offsets, layout) = self.compute_layout(&field_kinds);
        let mut structs = self.structs.write().unwrap();
        let id = structs.len() as u32;
        structs.push(StructEntry {
            name: name.to_string(),
            field_kinds,
            field_offsets,
            layout,
        });
        id
    }

    pub(super) fn push_enum(&self, name: &str, members: Vec<(String, i32)>) -> u32 {
        let mut enums = self.enum_entries.write().unwrap();
        let id = enums.len() as u32;
        enums.push(EnumData {
            name: name.to_string(),
            members,
        });
        id
    }

    // -----------------------------------------------------------------------
    // Interface method table writes
    // -----------------------------------------------------------------------

    /// Create an interface method table. Called only when dedup already checked by caller.
    /// If a method table for this IID already exists, its base MUST match;
    /// otherwise subsequent method registrations for the
    /// IID would compute wrong vtable indices for one of the callers.
    pub(super) fn create_interface_method_table(&self, iid: GUID, base_slot: usize) {
        assert!(matches!(base_slot, 3 | 6));
        let mut tables = self.interface_methods.write().unwrap();
        match tables.entry(iid) {
            std::collections::hash_map::Entry::Occupied(existing) => {
                let existing_base = existing.get().base_slot;
                assert_eq!(
                    existing_base, base_slot,
                    "interface IID {iid:?} registered twice with conflicting bases \
                     (existing={existing_base}, new={base_slot})"
                );
            }
            std::collections::hash_map::Entry::Vacant(v) => {
                v.insert(InterfaceMethodTable {
                    method_names: Vec::new(),
                    method_indices: Vec::new(),
                    base_slot,
                });
            }
        }
    }

    /// Add a method to an interface's method table. Returns the vtable index.
    /// If a method with the same name already exists, skips and returns the existing vtable index.
    pub(super) fn push_method(&self, iid: &GUID, name: &str, sig: MethodSignature) -> u32 {
        let mut iface_methods = self.interface_methods.write().unwrap();
        let table = iface_methods
            .get_mut(iid)
            .expect("Interface not found — call register_interface first");

        // Dedup: if method name already registered, return existing vtable index
        if let Some(pos) = table.method_names.iter().position(|n| n == name) {
            return (table.base_slot + pos) as u32;
        }

        let vtable_index = table.base_slot + table.method_indices.len();
        let method = sig.build(vtable_index);
        let arena_index = self.methods.push(method);
        table.method_names.push(name.to_string());
        table.method_indices.push(arena_index);
        vtable_index as u32
    }

    // -----------------------------------------------------------------------
    // Named type index (unified dedup for struct, enum, runtime_class, interface)
    // -----------------------------------------------------------------------

    pub(super) fn get_named_type(&self, name: &str) -> Option<TypeKind> {
        self.type_names.read().unwrap().get(name).copied()
    }

    pub(super) fn insert_named_type(&self, name: &str, kind: TypeKind) {
        self.type_names
            .write()
            .unwrap()
            .insert(name.to_string(), kind);
    }

    // -----------------------------------------------------------------------
    // Arena read operations
    // -----------------------------------------------------------------------

    pub(crate) fn get_inner_type(&self, idx: u32) -> TypeKind {
        self.inner_types.read().unwrap()[idx as usize]
    }

    pub(crate) fn get_inner_type_pair(&self, idx: u32) -> (TypeKind, TypeKind) {
        self.inner_type_pairs.read().unwrap()[idx as usize]
    }

    pub(crate) fn get_runtime_class(&self, idx: u32) -> (String, TypeKind) {
        let rcs = self.runtime_classes.read().unwrap();
        let rc = &rcs[idx as usize];
        (rc.name.clone(), rc.default_interface)
    }

    pub(crate) fn get_parameterized(&self, idx: u32) -> (TypeKind, Vec<TypeKind>) {
        let pts = self.parameterized_types.read().unwrap();
        let p = &pts[idx as usize];
        (p.generic_def, p.args.clone())
    }

    pub(super) fn get_enum_name(&self, idx: u32) -> String {
        self.enum_entries.read().unwrap()[idx as usize].name.clone()
    }

    pub(crate) fn get_enum_member_name(&self, idx: u32, value: i32) -> Option<String> {
        let enums = self.enum_entries.read().unwrap();
        let entry = &enums[idx as usize];
        entry
            .members
            .iter()
            .find(|(_, v)| *v == value)
            .map(|(n, _)| n.clone())
    }

    pub(super) fn get_enum_members(&self, enum_name: &str) -> Option<Vec<(String, i32)>> {
        let enums = self.enum_entries.read().unwrap();
        enums
            .iter()
            .find(|e| e.name == enum_name)
            .map(|e| e.members.clone())
    }

    pub(crate) fn invoke_method(
        &self,
        index: u32,
        obj: *mut std::ffi::c_void,
        args: &[WinRTValue],
    ) -> windows_core::Result<Vec<WinRTValue>> {
        // Grab a stable pointer to the method, then invoke. `AppendOnlyBoxArena`
        // guarantees the pointer remains valid for the arena's lifetime, which
        // lets us release the read lock before making the COM call. Holding the
        // read lock across dispatch would deadlock any re-entrant `push_method`
        // triggered from within (e.g. an event handler that touches a lazy
        // proxy while `runEventLoop` is pumping). See the doc comment on
        // `methods` in mod.rs for the safety argument.
        let method_ptr: *const crate::signature::Method = self.methods.stable_ptr(index);
        unsafe { (*method_ptr).call_dynamic(obj, args) }
    }

    /// Get a stable pointer to a Method by arena index. See the safety notes
    /// on `MetadataTable::methods` for why this is sound.
    pub(crate) fn method_ptr(&self, index: u32) -> *const crate::signature::Method {
        self.methods.stable_ptr(index)
    }

    pub(super) fn get_method_arena_index_by_vtable(
        &self,
        iid: &GUID,
        vtable_index: usize,
    ) -> Option<u32> {
        let iface_methods = self.interface_methods.read().unwrap();
        let table = iface_methods.get(iid)?;
        if vtable_index < table.base_slot {
            return None;
        }
        let local_index = vtable_index - table.base_slot;
        table.method_indices.get(local_index).copied()
    }

    pub(super) fn get_method_arena_index_by_name(&self, iid: &GUID, name: &str) -> Option<u32> {
        let iface_methods = self.interface_methods.read().unwrap();
        let table = iface_methods.get(iid)?;
        let pos = table.method_names.iter().position(|n| n == name)?;
        Some(table.method_indices[pos])
    }

    // -----------------------------------------------------------------------
    // Layout engine
    // -----------------------------------------------------------------------

    pub(crate) fn size_of_kind(&self, kind: TypeKind) -> usize {
        if let Some(s) = kind.primitive_size() {
            return s;
        }
        match kind {
            TypeKind::Struct(id) => self.structs.read().unwrap()[id as usize].layout.size(),
            TypeKind::HString
            | TypeKind::Object
            | TypeKind::Interface(_)
            | TypeKind::Delegate(_)
            | TypeKind::RuntimeClass(_)
            | TypeKind::Parameterized(_)
            | TypeKind::IAsyncAction
            | TypeKind::IAsyncActionWithProgress(_)
            | TypeKind::IAsyncOperation(_)
            | TypeKind::IAsyncOperationWithProgress(_)
            | TypeKind::OutValue(_)
            | TypeKind::ArrayOfIUnknown => std::mem::size_of::<*mut std::ffi::c_void>(),
            _ => panic!("size_of_kind not supported for {:?}", kind),
        }
    }

    pub(crate) fn align_of_kind(&self, kind: TypeKind) -> usize {
        if let Some(a) = kind.primitive_align() {
            return a;
        }
        match kind {
            TypeKind::Struct(id) => self.structs.read().unwrap()[id as usize].layout.align(),
            TypeKind::HString
            | TypeKind::Object
            | TypeKind::Interface(_)
            | TypeKind::Delegate(_)
            | TypeKind::RuntimeClass(_)
            | TypeKind::Parameterized(_)
            | TypeKind::IAsyncAction
            | TypeKind::IAsyncActionWithProgress(_)
            | TypeKind::IAsyncOperation(_)
            | TypeKind::IAsyncOperationWithProgress(_)
            | TypeKind::OutValue(_)
            | TypeKind::ArrayOfIUnknown => std::mem::align_of::<*mut std::ffi::c_void>(),
            _ => panic!("align_of_kind not supported for {:?}", kind),
        }
    }

    pub(crate) fn layout_of_kind(&self, kind: TypeKind) -> Layout {
        let size = self.size_of_kind(kind);
        let align = self.align_of_kind(kind);
        Layout::from_size_align(size, align).unwrap()
    }

    pub(crate) fn field_count_kind(&self, kind: TypeKind) -> usize {
        match kind {
            TypeKind::Struct(id) => self.structs.read().unwrap()[id as usize].field_kinds.len(),
            _ => panic!("field_count: type {:?} has no fields", kind),
        }
    }

    pub(crate) fn field_offset_kind(&self, kind: TypeKind, index: usize) -> usize {
        match kind {
            TypeKind::Struct(id) => self.structs.read().unwrap()[id as usize].field_offsets[index],
            _ => panic!("field_offset: type {:?} has no fields", kind),
        }
    }

    pub(crate) fn field_kind(&self, kind: TypeKind, index: usize) -> TypeKind {
        match kind {
            TypeKind::Struct(id) => self.structs.read().unwrap()[id as usize].field_kinds[index],
            _ => panic!("field_kind: type {:?} has no fields", kind),
        }
    }

    pub(crate) fn libffi_type_kind(&self, kind: TypeKind) -> libffi::middle::Type {
        if let Some(t) = kind.primitive_libffi_type() {
            return t;
        }
        match kind {
            TypeKind::Struct(id) => {
                let structs = self.structs.read().unwrap();
                let field_types: Vec<libffi::middle::Type> = structs[id as usize]
                    .field_kinds
                    .iter()
                    .map(|f| self.libffi_type_kind(*f))
                    .collect();
                libffi::middle::Type::structure(field_types)
            }
            // Pointer-sized types (COM objects, HString handle, etc.)
            TypeKind::HString
            | TypeKind::Object
            | TypeKind::Interface(_)
            | TypeKind::Delegate(_)
            | TypeKind::RuntimeClass(_)
            | TypeKind::Parameterized(_)
            | TypeKind::IAsyncAction
            | TypeKind::IAsyncActionWithProgress(_)
            | TypeKind::IAsyncOperation(_)
            | TypeKind::IAsyncOperationWithProgress(_)
            | TypeKind::OutValue(_)
            | TypeKind::ArrayOfIUnknown => libffi::middle::Type::pointer(),
            _ => panic!("libffi_type_kind: unsupported for {:?}", kind),
        }
    }

    pub(super) fn compute_layout(&self, fields: &[TypeKind]) -> (Vec<usize>, Layout) {
        let mut offsets = Vec::with_capacity(fields.len());
        let mut layout = Layout::from_size_align(0, 1).unwrap();

        for field in fields {
            let field_layout = self.layout_of_kind(*field);
            let (new_layout, offset) = layout.extend(field_layout).unwrap();
            offsets.push(offset);
            layout = new_layout;
        }

        (offsets, layout.pad_to_align())
    }
}
