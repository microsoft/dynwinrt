// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use crate::system_helpers::test_support::{ResolutionFailure, delete_calls, resolution_calls};
use windows::Win32::{
    Foundation::{E_ACCESSDENIED, ERROR_PROC_NOT_FOUND},
    Graphics::Gdi::{BITMAP, CreateBitmap, GetObjectW, HGDIOBJ},
};

#[repr(C)]
struct BitmapCall {
    vtable: *const *mut c_void,
    status: HRESULT,
    calls: Cell<usize>,
    output: Cell<*mut c_void>,
    cleanup_phase: RefCell<Option<ResolutionFailure>>,
}

impl BitmapCall {
    fn new(vtable: &[*mut c_void], status: HRESULT) -> Self {
        Self {
            vtable: vtable.as_ptr(),
            status,
            calls: Cell::new(0),
            output: Cell::new(std::ptr::null_mut()),
            cleanup_phase: RefCell::new(None),
        }
    }

    fn produce(&self) -> *mut c_void {
        self.calls.set(self.calls.get() + 1);
        let bitmap = unsafe { CreateBitmap(2, 2, 1, 1, None) };
        self.output.set(bitmap.0);
        // Any attempt to resolve again after native dispatch must fail. Cleanup
        // must use the deleter secured before the callee acquired ownership.
        *self.cleanup_phase.borrow_mut() = Some(ResolutionFailure::new(HRESULT::from_win32(
            ERROR_PROC_NOT_FOUND.0,
        )));
        bitmap.0
    }

    fn assert_cleaned_once(&self, before: usize) {
        assert_eq!(self.calls.get(), 1);
        assert!(!self.output.get().is_null());
        assert_eq!(delete_calls(), before + 1);
        let mut info = BITMAP::default();
        assert_eq!(
            unsafe {
                GetObjectW(
                    HGDIOBJ(self.output.get()),
                    size_of::<BITMAP>() as i32,
                    Some((&mut info as *mut BITMAP).cast()),
                )
            },
            0
        );
    }
}

unsafe extern "system" fn write_bitmap(this: *mut c_void, output: *mut *mut c_void) -> HRESULT {
    let call = unsafe { &*this.cast::<BitmapCall>() };
    unsafe { *output = call.produce() };
    call.status
}

unsafe extern "system" fn write_bitmap_with_input(
    this: *mut c_void,
    _: u32,
    output: *mut *mut c_void,
) -> HRESULT {
    unsafe { write_bitmap(this, output) }
}

unsafe extern "system" fn write_bitmap_and_number(
    this: *mut c_void,
    output: *mut *mut c_void,
    number: *mut u32,
) -> HRESULT {
    unsafe { *number = 42 };
    unsafe { write_bitmap(this, output) }
}

unsafe extern "system" fn return_bitmap(this: *mut c_void) -> *mut c_void {
    unsafe { &*this.cast::<BitmapCall>() }.produce()
}

fn owned() -> Type {
    Type::owned_handle_output(OwnedHandleCleanup::DeleteObject)
}

fn output_cases(table: &Arc<MetadataTable>) -> Vec<(MethodSignature, *mut c_void, Vec<Value>)> {
    vec![
        (
            MethodSignature::new(table).add_out(owned()),
            write_bitmap as *mut c_void,
            vec![],
        ),
        (
            MethodSignature::new(table)
                .add_in(Type::winrt(table.u32_type()))
                .add_out(owned()),
            write_bitmap_with_input as *mut c_void,
            vec![Value::WinRt(WinRTValue::U32(7))],
        ),
        (
            MethodSignature::new(table)
                .add_out(owned())
                .add_out(Type::winrt(table.u32_type())),
            write_bitmap_and_number as *mut c_void,
            vec![],
        ),
        (
            MethodSignature::new(table).add_optional_out(owned()),
            write_bitmap as *mut c_void,
            vec![Value::WinRt(WinRTValue::Bool(true))],
        ),
        (
            MethodSignature::new(table)
                .add_out(owned())
                .preserve_hresult(),
            write_bitmap as *mut c_void,
            vec![],
        ),
    ]
}

#[test]
fn gdi_cleanup_admission_is_lazy_retryable_and_precedes_every_dispatch_path() {
    let table = MetadataTable::new();
    let mut cases = output_cases(&table);
    cases.push((
        MethodSignature::new(&table).returns(owned()),
        return_bitmap as *mut c_void,
        vec![],
    ));
    for (signature, function, args) in cases {
        let denied = ResolutionFailure::new(HRESULT::from_win32(ERROR_PROC_NOT_FOUND.0));
        // Signature lowering/registration cannot perform DLL resolution.
        let resolutions = resolution_calls();
        let registered = signature.build(0).unwrap();
        assert_eq!(resolution_calls(), resolutions);
        let vtable = [function];
        let mut call = BitmapCall::new(&vtable, HRESULT(0));
        let dispatched = Cell::new(false);
        let before = delete_calls();
        let error = registered
            .plan
            .invoke_values_guarded((&mut call as *mut BitmapCall).cast(), &args, || {
                dispatched.set(true);
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(error, result::Error::WindowsError(ref error)
            if error.code() == HRESULT::from_win32(ERROR_PROC_NOT_FOUND.0)));
        assert!(!dispatched.get());
        assert_eq!(call.calls.get(), 0);
        assert!(call.output.get().is_null());
        assert_eq!(delete_calls(), before);
        drop(denied);

        let values = registered
            .plan
            .invoke_values((&mut call as *mut BitmapCall).cast(), &args)
            .unwrap();
        let pointer = values
            .iter()
            .find_map(|value| match value {
                Value::WinRt(WinRTValue::RawPtr(pointer)) => Some(*pointer),
                _ => None,
            })
            .unwrap();
        assert_eq!(pointer, call.output.get());
        unsafe { OutputCleanup::DeleteObject.cleanup(pointer) };
        call.assert_cleaned_once(before);
    }
}

unsafe extern "system" fn write_null(_: *mut c_void, output: *mut *mut c_void) -> HRESULT {
    unsafe { *output = std::ptr::null_mut() };
    HRESULT(0)
}

#[test]
fn borrowed_handle_calls_do_not_resolve_gdi_cleanup() {
    let _denied = ResolutionFailure::new(HRESULT::from_win32(ERROR_PROC_NOT_FOUND.0));
    let before = resolution_calls();
    let registered = MethodSignature::new(&MetadataTable::new())
        .add_out(Type::borrowed_handle_output())
        .build(0)
        .unwrap();
    let vtable = [write_null as *mut c_void];
    let mut call = BitmapCall::new(&vtable, HRESULT(0));
    let values = registered
        .plan
        .invoke_values((&mut call as *mut BitmapCall).cast(), &[])
        .unwrap();
    assert!(
        matches!(values.as_slice(), [Value::WinRt(WinRTValue::RawPtr(pointer))] if pointer.is_null())
    );
    assert_eq!(resolution_calls(), before);
}

#[test]
fn gdi_partial_outputs_are_cleaned_once_without_replacing_failed_hresult() {
    for (signature, function, args) in output_cases(&MetadataTable::new()) {
        let registered = signature.build(0).unwrap();
        let vtable = [function];
        let mut call = BitmapCall::new(&vtable, E_ACCESSDENIED);
        let before = delete_calls();
        let error = registered
            .plan
            .invoke_values((&mut call as *mut BitmapCall).cast(), &args)
            .unwrap_err();
        assert!(
            matches!(error, result::Error::WindowsError(ref error) if error.code() == E_ACCESSDENIED)
        );
        call.assert_cleaned_once(before);
        drop(error);
        assert_eq!(delete_calls(), before + 1);
    }
}

#[repr(C)]
struct SizedPod {
    cb_size: u32,
    value: u32,
}

fn sized_pod() -> Type {
    let layout = NativeStructLayout::new(
        "Tests.GdiCleanupSizedPod",
        size_of::<SizedPod>(),
        align_of::<SizedPod>(),
        vec![
            NativeStructField::new(
                "cbSize",
                0,
                1,
                NativeStructFieldType::Scalar(NativeStructScalar::U32),
            )
            .unwrap(),
            NativeStructField::new(
                "value",
                4,
                1,
                NativeStructFieldType::Scalar(NativeStructScalar::U32),
            )
            .unwrap(),
        ],
    )
    .unwrap()
    .with_size_field_initializer("cbSize")
    .unwrap();
    Type::raw_native_struct(Arc::new(layout)).unwrap()
}

unsafe extern "system" fn invalid_return_with_bitmap(
    this: *mut c_void,
    bitmap: *mut *mut c_void,
) -> SizedPod {
    unsafe { *bitmap = (&*this.cast::<BitmapCall>()).produce() };
    SizedPod {
        cb_size: 0,
        value: 42,
    }
}

unsafe extern "system" fn bitmap_return_with_invalid_output(
    this: *mut c_void,
    pod: *mut SizedPod,
) -> *mut c_void {
    unsafe {
        *pod = SizedPod {
            cb_size: 0,
            value: 42,
        }
    };
    unsafe { &*this.cast::<BitmapCall>() }.produce()
}

#[test]
fn gdi_direct_and_parameter_outputs_survive_fallible_result_extraction() {
    let table = MetadataTable::new();
    for (signature, function) in [
        (
            MethodSignature::new(&table)
                .add_out(owned())
                .returns(sized_pod()),
            invalid_return_with_bitmap as *mut c_void,
        ),
        (
            MethodSignature::new(&table)
                .add_out(sized_pod())
                .returns(owned()),
            bitmap_return_with_invalid_output as *mut c_void,
        ),
    ] {
        let registered = signature.build(0).unwrap();
        let vtable = [function];
        let mut call = BitmapCall::new(&vtable, HRESULT(0));
        let before = delete_calls();
        let error = registered
            .plan
            .invoke_values((&mut call as *mut BitmapCall).cast(), &[])
            .unwrap_err();
        assert!(error.message().contains("size field"));
        call.assert_cleaned_once(before);
    }
}

unsafe extern "system" fn bitmap_and_invalid_buffer_count(
    this: *mut c_void,
    bitmap: *mut *mut c_void,
    _: *mut u8,
    capacity: u32,
    actual: *mut u32,
) -> HRESULT {
    unsafe {
        *bitmap = (&*this.cast::<BitmapCall>()).produce();
        *actual = capacity + 1;
    }
    HRESULT(0)
}

#[test]
fn gdi_output_is_cleaned_when_com_post_call_validation_fails() {
    let table = MetadataTable::new();
    let registered = MethodSignature::new(&table)
        .add_out(owned())
        .add_caller_output_buffer(
            Type::winrt(table.u8_type()),
            2,
            Some(3),
            BufferCountUnit::Elements,
            false,
        )
        .unwrap()
        .add_in(Type::winrt(table.u32_type()))
        .add_out(Type::winrt(table.u32_type()))
        .build(0)
        .unwrap();
    let vtable = [bitmap_and_invalid_buffer_count as *mut c_void];
    let mut call = BitmapCall::new(&vtable, HRESULT(0));
    let mut bytes = [0u8; 4];
    let buffer =
        unsafe { ComBufferValue::borrowed(bytes.as_mut_ptr().cast(), bytes.len(), 1, true, true) }
            .unwrap();
    let before = delete_calls();
    let error = registered
        .plan
        .invoke_values(
            (&mut call as *mut BitmapCall).cast(),
            &[Value::Buffer(buffer)],
        )
        .unwrap_err();
    assert!(error.message().contains("exceeds capacity"));
    call.assert_cleaned_once(before);
}
