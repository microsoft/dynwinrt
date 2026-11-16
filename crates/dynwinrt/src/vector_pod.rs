// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::LazyLock;

use libffi::middle::Type;

use super::*;
use crate::{
    TypeHandle,
    native_callback::{CallbackAbiType, CallbackSignature, callback_code},
};

pub(super) struct PodVectorVtables {
    pub vector: VectorVtbl,
    pub live: VectorViewVtbl,
    pub snapshot: VectorViewVtbl,
}

// Tables contain only immutable function pointers. The shared callback cache
// owns the executable pages and prepared CIF/type graphs for the process lifetime.
unsafe impl Send for PodVectorVtables {}
unsafe impl Sync for PodVectorVtables {}

static TABLES: LazyLock<Mutex<HashMap<String, Arc<PodVectorVtables>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn preparation_error(message: String) -> crate::Error {
    windows_core::Error::new(E_FAIL, &message).into()
}

impl PodVectorVtables {
    // Called only after CollectionElementPlan validates the complete POD layout.
    pub fn for_type(typ: &TypeHandle) -> crate::Result<Arc<Self>> {
        let signature = typ.signature_string();
        let mut tables = TABLES
            .lock()
            .map_err(|_| preparation_error("POD vector vtable cache is poisoned".into()))?;
        if let Some(tables) = tables.get(&signature) {
            return Ok(tables.clone());
        }
        let element = (
            CallbackAbiType::NativeStruct(signature.clone(), typ.size_of()),
            typ.libffi_type(),
        );
        let pointer = || (CallbackAbiType::Pointer, Type::pointer());
        let index = || (CallbackAbiType::U32, Type::u32());
        let lookup = CallbackSignature::hresult(vec![element.clone(), pointer(), pointer()]);
        let indexed = CallbackSignature::hresult(vec![index(), element.clone()]);
        let append = CallbackSignature::hresult(vec![element]);
        let vector = VectorVtbl {
            index_of: unsafe {
                std::mem::transmute(
                    callback_code(9, lookup.clone(), vector_dispatch).map_err(preparation_error)?,
                )
            },
            set_at: unsafe {
                std::mem::transmute(
                    callback_code(10, indexed.clone(), vector_dispatch)
                        .map_err(preparation_error)?,
                )
            },
            insert_at: unsafe {
                std::mem::transmute(
                    callback_code(11, indexed, vector_dispatch).map_err(preparation_error)?,
                )
            },
            append: unsafe {
                std::mem::transmute(
                    callback_code(13, append, vector_dispatch).map_err(preparation_error)?,
                )
            },
            ..SingleThreadedVector::VECTOR_VTBL
        };
        let live = VectorViewVtbl {
            index_of: unsafe {
                std::mem::transmute(
                    callback_code(8, lookup.clone(), live_dispatch).map_err(preparation_error)?,
                )
            },
            ..SingleThreadedVector::VIEW_VTBL
        };
        let snapshot = VectorViewVtbl {
            index_of: unsafe {
                std::mem::transmute(
                    callback_code(8, lookup, snapshot_dispatch).map_err(preparation_error)?,
                )
            },
            ..SingleThreadedVectorView::VIEW_VTBL
        };
        let plan = Arc::new(Self {
            vector,
            live,
            snapshot,
        });
        tables.insert(signature, plan.clone());
        Ok(plan)
    }
}

unsafe fn argument<T: Copy>(args: *const *const c_void, index: usize) -> T {
    unsafe { (*args.add(index)).cast::<T>().read_unaligned() }
}

unsafe fn vector_dispatch(
    slot: usize,
    _signature: &CallbackSignature,
    args: *const *const c_void,
    result: *mut c_void,
) {
    let this = unsafe { argument(args, 0) };
    // libffi presents by-value aggregates as addresses of complete native
    // storage, independently of register/HFA/indirect/stack classification.
    let hr = unsafe {
        match slot {
            9 => SingleThreadedVector::index_of_impl(
                this,
                (*args.add(1)).cast_mut(),
                argument(args, 2),
                argument(args, 3),
            ),
            10 => SingleThreadedVector::set_at_impl(
                this,
                argument(args, 1),
                (*args.add(2)).cast_mut(),
            ),
            11 => SingleThreadedVector::insert_at_impl(
                this,
                argument(args, 1),
                (*args.add(2)).cast_mut(),
            ),
            13 => SingleThreadedVector::append_impl(this, (*args.add(1)).cast_mut()),
            _ => unreachable!("unpublished POD vector slot"),
        }
    };
    unsafe { result.cast::<i32>().write(hr.0) };
}

unsafe fn live_dispatch(
    _slot: usize,
    _signature: &CallbackSignature,
    args: *const *const c_void,
    result: *mut c_void,
) {
    let hr = unsafe {
        SingleThreadedVector::view_index_of_impl(
            argument(args, 0),
            (*args.add(1)).cast_mut(),
            argument(args, 2),
            argument(args, 3),
        )
    };
    unsafe { result.cast::<i32>().write(hr.0) };
}

unsafe fn snapshot_dispatch(
    _slot: usize,
    _signature: &CallbackSignature,
    args: *const *const c_void,
    result: *mut c_void,
) {
    let hr = unsafe {
        SingleThreadedVectorView::index_of_impl(
            argument(args, 0),
            (*args.add(1)).cast_mut(),
            argument(args, 2),
            argument(args, 3),
        )
    };
    unsafe { result.cast::<i32>().write(hr.0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn immutable_pod_tables_are_cached_without_retaining_metadata() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<PodVectorVtables>();
        let table = crate::MetadataTable::new();
        let typ = table.struct_type("Test.CachedPod", &vec![table.f64_type(); 3]);
        let plan = PodVectorVtables::for_type(&typ).unwrap();
        let second = PodVectorVtables::for_type(&typ).unwrap();
        assert!(Arc::ptr_eq(&plan, &second));
        let other = crate::MetadataTable::new();
        let equivalent = other.struct_type("Test.CachedPod", &vec![other.f64_type(); 3]);
        assert!(Arc::ptr_eq(
            &plan,
            &PodVectorVtables::for_type(&equivalent).unwrap()
        ));
        let weak = Arc::downgrade(&table);
        drop(typ);
        drop(table);
        assert!(weak.upgrade().is_none());
    }

    #[test]
    fn concurrent_pod_preparation_shares_one_immutable_table() {
        const WORKERS: usize = 8;
        let barrier = Arc::new(std::sync::Barrier::new(WORKERS));
        let workers = (0..WORKERS)
            .map(|_| {
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let table = crate::MetadataTable::new();
                    let weak = Arc::downgrade(&table);
                    let typ = table.struct_type("Test.ConcurrentPod", &vec![table.f64_type(); 3]);
                    barrier.wait();
                    let plan = PodVectorVtables::for_type(&typ).unwrap();
                    drop(typ);
                    drop(table);
                    assert!(weak.upgrade().is_none());
                    plan
                })
            })
            .collect::<Vec<_>>();
        let plans = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        for plan in &plans[1..] {
            assert!(Arc::ptr_eq(&plans[0], plan));
        }
    }

    #[test]
    fn same_size_pods_with_different_field_abis_do_not_share_callbacks() {
        let floating = crate::MetadataTable::new();
        let floating_type = floating.struct_type("Test.SameSizePod", &vec![floating.f64_type(); 2]);
        let integer = crate::MetadataTable::new();
        let integer_type = integer.struct_type("Test.SameSizePod", &vec![integer.u64_type(); 2]);
        assert_eq!(floating_type.size_of(), integer_type.size_of());
        assert_eq!(floating_type.align_of(), integer_type.align_of());
        let floating = PodVectorVtables::for_type(&floating_type).unwrap();
        let integer = PodVectorVtables::for_type(&integer_type).unwrap();
        assert!(!Arc::ptr_eq(&floating, &integer));
        assert_ne!(
            floating.vector.append as usize,
            integer.vector.append as usize
        );
        assert_ne!(
            floating.live.index_of as usize,
            integer.live.index_of as usize
        );
        assert_ne!(
            floating.snapshot.index_of as usize,
            integer.snapshot.index_of as usize
        );
    }
}
