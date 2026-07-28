// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::future::{Future, IntoFuture};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};

use windows::core::Interface;
use windows_core::{GUID, HRESULT, IUnknown};
use windows_future::{AsyncActionCompletedHandler, AsyncStatus};

use crate::metadata_table::IASYNC_ACTION;
use crate::result::{Error, Result};
use crate::value::WinRTValue;

// ---------------------------------------------------------------------------
// DynCompletedHandler — a minimal COM object for WinRT completion callbacks
// Used for generic async types (IAsyncOperation<T>, etc.) where the handler
// IID is parameterized and we can't use windows-future's typed handlers.
// ---------------------------------------------------------------------------

#[repr(C)]
struct DynCompletedHandlerVtbl {
    base: windows_core::IUnknown_Vtbl,
    invoke: unsafe extern "system" fn(
        this: *mut std::ffi::c_void,
        sender: *mut std::ffi::c_void,
        status: AsyncStatus,
    ) -> HRESULT,
}

#[repr(C)]
struct DynCompletedHandler {
    vtable: *const DynCompletedHandlerVtbl,
    ref_count: windows_core::imp::RefCount,
    handler_iid: GUID,
    callback: Arc<dyn Fn() + Send + Sync>,
}

impl DynCompletedHandler {
    const VTBL: DynCompletedHandlerVtbl = DynCompletedHandlerVtbl {
        base: windows_core::IUnknown_Vtbl {
            QueryInterface: Self::qi,
            AddRef: Self::add_ref,
            Release: Self::release,
        },
        invoke: Self::invoke,
    };

    fn create(callback: Arc<dyn Fn() + Send + Sync>, handler_iid: GUID) -> IUnknown {
        let handler = Box::new(Self {
            vtable: &Self::VTBL,
            ref_count: windows_core::imp::RefCount::new(1),
            handler_iid,
            callback,
        });
        unsafe { IUnknown::from_raw(Box::into_raw(handler) as *mut std::ffi::c_void) }
    }

    unsafe extern "system" fn qi(
        this: *mut std::ffi::c_void,
        iid: *const GUID,
        ppv: *mut *mut std::ffi::c_void,
    ) -> HRESULT {
        if iid.is_null() || ppv.is_null() {
            return HRESULT(-2147467261); // E_INVALIDARG
        }
        let iid = unsafe { &*iid };
        let handler = unsafe { &*(this as *const Self) };
        if *iid == IUnknown::IID
            || *iid == windows_core::imp::IAgileObject::IID
            || *iid == handler.handler_iid
        {
            unsafe { *ppv = this };
            unsafe { Self::add_ref(this) };
            HRESULT(0) // S_OK
        } else if *iid == windows_core::imp::IMarshal::IID {
            unsafe {
                handler.ref_count.add_ref();
                windows_core::imp::marshaler(core::mem::transmute(this), ppv)
            }
        } else {
            unsafe { *ppv = std::ptr::null_mut() };
            HRESULT(-2147467262) // E_NOINTERFACE
        }
    }

    unsafe extern "system" fn add_ref(this: *mut std::ffi::c_void) -> u32 {
        let handler = unsafe { &*(this as *const Self) };
        handler.ref_count.add_ref()
    }

    unsafe extern "system" fn release(this: *mut std::ffi::c_void) -> u32 {
        let handler = unsafe { &*(this as *const Self) };
        let remaining = handler.ref_count.release();
        if remaining == 0 {
            unsafe { drop(Box::from_raw(this as *mut Self)) };
        }
        remaining
    }

    unsafe extern "system" fn invoke(
        this: *mut std::ffi::c_void,
        _sender: *mut std::ffi::c_void,
        _status: AsyncStatus,
    ) -> HRESULT {
        let handler = unsafe { &*(this as *const Self) };
        (handler.callback)();
        HRESULT(0) // S_OK
    }
}

// ---------------------------------------------------------------------------
// WinRTAsyncFuture — event-driven Future for dynamic WinRT async operations
// ---------------------------------------------------------------------------

use crate::value::AsyncInfo;

pub struct WinRTAsyncFuture {
    async_info: AsyncInfo,
    waker: Option<Arc<Mutex<Waker>>>,
    cancel_on_drop: bool,
    defer_get_results: bool,
}

// WinRT async operations are agile objects and safe to send across threads.
unsafe impl Send for WinRTAsyncFuture {}

impl WinRTAsyncFuture {
    fn from_value(value: WinRTValue) -> Self {
        match value {
            WinRTValue::Async(a) => Self {
                async_info: a,
                waker: None,
                cancel_on_drop: false,
                defer_get_results: false,
            },
            _ => panic!("WinRTAsyncFuture::from_value called with non-async WinRTValue"),
        }
    }

    fn from_async_info(info: AsyncInfo) -> Self {
        Self {
            async_info: info,
            waker: None,
            cancel_on_drop: false,
            defer_get_results: false,
        }
    }

    /// Cancel the underlying WinRT operation if this future is dropped while pending.
    pub fn cancel_on_drop(mut self) -> Self {
        self.cancel_on_drop = true;
        self
    }

    /// Complete with the async handle so GetResults can run on the consuming thread.
    pub fn defer_get_results(mut self) -> Self {
        self.defer_get_results = true;
        self
    }

    /// QI from IAsyncInfo to the concrete async interface.
    fn query_concrete(&self) -> Result<IUnknown> {
        let iid = self.async_info.iid();
        let mut ptr = std::ptr::null_mut();
        unsafe { self.async_info.info.query(&iid, &mut ptr) }
            .ok()
            .map_err(Error::WindowsError)?;
        Ok(unsafe { IUnknown::from_raw(ptr) })
    }

    /// Vtable indices for SetCompleted and GetResults.
    fn vtable_indices(&self) -> (usize, usize) {
        use crate::metadata_table::TypeKind;
        match self.async_info.async_type.kind() {
            TypeKind::IAsyncAction | TypeKind::IAsyncOperation(_) => (6, 8),
            TypeKind::IAsyncActionWithProgress(_) | TypeKind::IAsyncOperationWithProgress(_) => {
                (8, 10)
            }
            _ => panic!("not an async type"),
        }
    }

    /// Call GetResults on the concrete async interface and return the WinRTValue.
    fn get_results(&self) -> Result<WinRTValue> {
        let concrete = self.query_concrete()?;
        let (_, get_results_index) = self.vtable_indices();

        if let Some(rt) = self.async_info.result_type() {
            let mut out = rt.default_winrt_value();
            let hr = crate::call::call_winrt_method_1(
                get_results_index,
                concrete.as_raw(),
                out.out_ptr(),
            );
            hr.ok().map_err(Error::WindowsError)?;
            // Pointer types use RawPtr(null) as buffer; convert via from_out.
            if let WinRTValue::RawPtr(raw_ptr) = out {
                out = rt.from_out(raw_ptr)?;
            }

            out.sanitize_null_object();
            Ok(out)
        } else {
            let mut dummy: *mut std::ffi::c_void = std::ptr::null_mut();
            let hr =
                crate::call::call_winrt_method_1(get_results_index, concrete.as_raw(), &mut dummy);
            hr.ok().map_err(Error::WindowsError)?;
            Ok(WinRTValue::HResult(HRESULT(0)))
        }
    }

    fn completed_value(&self) -> Result<WinRTValue> {
        if self.defer_get_results {
            Ok(WinRTValue::Async(self.async_info.clone()))
        } else {
            self.get_results()
        }
    }

    /// Register SetCompleted using the typed windows-future API (IAsyncAction)
    /// or via dynamic vtable call (generic types).
    fn register_completed(&self, shared_waker: Arc<Mutex<Waker>>) -> Result<()> {
        if self.async_info.iid() == IASYNC_ACTION {
            // IAsyncAction — use windows-future's typed handler directly
            let action: windows_future::IAsyncAction =
                self.async_info.info.cast().map_err(Error::WindowsError)?;
            let handler = AsyncActionCompletedHandler::new(move |_, _| {
                if let Ok(waker) = shared_waker.lock() {
                    waker.wake_by_ref();
                }
                Ok(())
            });
            action.SetCompleted(&handler).map_err(Error::WindowsError)?;
        } else {
            // Generic types — use DynCompletedHandler via vtable
            let handler = DynCompletedHandler::create(
                Arc::new(move || {
                    if let Ok(waker) = shared_waker.lock() {
                        waker.wake_by_ref();
                    }
                }),
                self.async_info.handler_iid(),
            );
            let concrete = self.query_concrete()?;
            let (set_completed_index, _) = self.vtable_indices();
            let hr = crate::call::call_winrt_method_1(
                set_completed_index,
                concrete.as_raw(),
                handler.as_raw(),
            );
            hr.ok().map_err(Error::WindowsError)?;
        }
        Ok(())
    }
}

pub type AsyncCompletedCallback = Box<dyn Fn() + Send + Sync>;

pub fn set_async_completed_handler(
    value: &WinRTValue,
    callback: AsyncCompletedCallback,
) -> Result<()> {
    let async_info = match value {
        WinRTValue::Async(info) => info.clone(),
        other => return Err(Error::ExpectedAsync(other.get_type_kind())),
    };
    let future = WinRTAsyncFuture::from_async_info(async_info);
    let handler = DynCompletedHandler::create(Arc::from(callback), future.async_info.handler_iid());
    let concrete = future.query_concrete()?;
    let (set_completed_index, _) = future.vtable_indices();
    let hr =
        crate::call::call_winrt_method_1(set_completed_index, concrete.as_raw(), handler.as_raw());
    hr.ok().map_err(Error::WindowsError)
}

impl Future for WinRTAsyncFuture {
    type Output = Result<WinRTValue>;

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Fast path: already completed before first poll
        match self.async_info.info.Status() {
            Ok(AsyncStatus::Canceled) => {
                self.cancel_on_drop = false;
                return Poll::Ready(Err(Error::Canceled));
            }
            Ok(status) if status != AsyncStatus::Started => {
                let result = self.completed_value();
                self.cancel_on_drop = false;
                return Poll::Ready(result);
            }
            Err(e) => {
                return Poll::Ready(Err(Error::WindowsError(e)));
            }
            _ => {}
        }

        if let Some(shared_waker) = &self.waker {
            // Subsequent poll — update the waker in case executor changed
            if let Ok(mut guard) = shared_waker.lock() {
                guard.clone_from(cx.waker());
            }
            // Re-check status (race: completion may have fired between status check and here)
            match self.async_info.info.Status() {
                Ok(AsyncStatus::Canceled) => {
                    self.cancel_on_drop = false;
                    return Poll::Ready(Err(Error::Canceled));
                }
                Ok(status) if status != AsyncStatus::Started => {
                    let result = self.completed_value();
                    self.cancel_on_drop = false;
                    return Poll::Ready(result);
                }

                Err(e) => {
                    return Poll::Ready(Err(Error::WindowsError(e)));
                }
                _ => {}
            }
        } else {
            // First poll — register SetCompleted
            let shared_waker = Arc::new(Mutex::new(cx.waker().clone()));
            self.waker = Some(shared_waker.clone());

            if let Err(e) = self.register_completed(shared_waker) {
                return Poll::Ready(Err(e));
            }
        }

        Poll::Pending
    }
}

pub fn get_async_results(value: &WinRTValue) -> Result<WinRTValue> {
    match value {
        WinRTValue::Async(info) => WinRTAsyncFuture::from_async_info(info.clone()).get_results(),
        other => Err(Error::ExpectedAsync(other.get_type_kind())),
    }
}

impl Drop for WinRTAsyncFuture {
    fn drop(&mut self) {
        if self.cancel_on_drop {
            let _ = self.async_info.cancel();
        }
    }
}

// ---------------------------------------------------------------------------
// IntoFuture for WinRTValue
// ---------------------------------------------------------------------------

impl IntoFuture for WinRTValue {
    type Output = Result<WinRTValue>;
    type IntoFuture = WinRTAsyncFuture;

    fn into_future(self) -> WinRTAsyncFuture {
        WinRTAsyncFuture::from_value(self)
    }
}

impl IntoFuture for &WinRTValue {
    type Output = Result<WinRTValue>;
    type IntoFuture = WinRTAsyncFuture;

    fn into_future(self) -> WinRTAsyncFuture {
        match self {
            WinRTValue::Async(a) => WinRTAsyncFuture::from_async_info(a.clone()),
            _ => panic!("IntoFuture for &WinRTValue called with non-async WinRTValue"),
        }
    }
}

// ---------------------------------------------------------------------------
// Progress handler — reuses delegate infrastructure
// ---------------------------------------------------------------------------

use crate::metadata_table::TypeHandle;

/// Callback type for progress notifications.
pub type ProgressCallback = Box<dyn Fn(WinRTValue) + Send + Sync>;

/// Create a progress handler for a WithProgress async operation.
///
/// Reuses `delegate::create_delegate` — the progress handler is simply a
/// COM delegate with `Invoke(sender, progress_value)`.
///
/// - `handler_iid`: parameterized IID of the progress handler delegate
/// - `progress_type`: TypeHandle for the progress value type
/// - `callback`: called on each progress notification (may be called from a WinRT background thread)
pub fn create_progress_handler(
    handler_iid: GUID,
    progress_type: TypeHandle,
    callback: ProgressCallback,
) -> IUnknown {
    // Progress handler Invoke signature: (sender: Object, progress: TProgress)
    let sender_type = progress_type
        .table()
        .make(crate::metadata_table::TypeKind::Object);
    let param_types = vec![sender_type, progress_type];

    let delegate_callback: crate::delegate::DelegateCallback =
        Box::new(move |args: &[WinRTValue]| {
            // args[0] = sender, args[1] = progress value
            if args.len() >= 2 {
                callback(args[1].clone());
            }
            HRESULT(0)
        });

    crate::delegate::create_delegate(handler_iid, param_types, delegate_callback)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use windows::System::Threading::{ThreadPool, WorkItemHandler};
    use windows::core::Interface;
    use windows_future::{AsyncStatus, IAsyncInfo};

    use crate::metadata_table::MetadataTable;
    use crate::result::{Error, Result};
    use crate::value::{AsyncInfo, WinRTValue};

    /// Verify SetProgress is at the same vtable offset for both WithProgress types.
    #[test]
    fn test_set_progress_vtable_offset_matches() {
        use windows_future::{IAsyncActionWithProgress_Vtbl, IAsyncOperationWithProgress_Vtbl};

        let action_offset = std::mem::offset_of!(IAsyncActionWithProgress_Vtbl<u32>, SetProgress);
        let operation_offset =
            std::mem::offset_of!(IAsyncOperationWithProgress_Vtbl<u64, u32>, SetProgress);

        assert_eq!(
            action_offset, operation_offset,
            "SetProgress vtable offset mismatch: ActionWithProgress={} vs OperationWithProgress={}",
            action_offset, operation_offset
        );

        // Also verify it's at index 6: 6 function pointers * pointer_size
        let expected = 6 * std::mem::size_of::<usize>();
        assert_eq!(
            action_offset, expected,
            "SetProgress should be at vtable index 6 (offset {}), got {}",
            expected, action_offset
        );

        println!(
            "SetProgress offset: {} (vtable index 6) -- both types match",
            action_offset
        );
    }

    #[tokio::test]
    async fn test_async_action() -> Result<()> {
        // ThreadPool.RunAsync returns IAsyncAction (no type parameters)
        let handler = WorkItemHandler::new(|_| Ok(()));
        let op = ThreadPool::RunAsync(&handler).map_err(Error::WindowsError)?;
        let async_info: IAsyncInfo = op.cast().map_err(Error::WindowsError)?;

        let reg = MetadataTable::new();
        let value = WinRTValue::Async(AsyncInfo {
            info: async_info,
            async_type: reg.async_action(),
        });
        let _result = value.await?;
        println!("IAsyncAction completed successfully");
        Ok(())
    }

    #[tokio::test]
    async fn test_deferred_get_results_keeps_async_handle() -> Result<()> {
        let handler = WorkItemHandler::new(|_| Ok(()));
        let operation = ThreadPool::RunAsync(&handler).map_err(Error::WindowsError)?;
        let info: IAsyncInfo = operation.cast().map_err(Error::WindowsError)?;
        let value = WinRTValue::Async(AsyncInfo {
            info,
            async_type: MetadataTable::new().async_action(),
        });

        let completed = value.into_future().defer_get_results().await?;
        assert!(matches!(completed, WinRTValue::Async(_)));
        assert!(matches!(
            super::get_async_results(&completed)?,
            WinRTValue::HResult(windows_core::HRESULT(0))
        ));
        Ok(())
    }

    #[test]
    fn test_completion_handler_notifies_without_executor_polling() -> Result<()> {
        let handler = WorkItemHandler::new(|_| Ok(()));
        let operation = ThreadPool::RunAsync(&handler).map_err(Error::WindowsError)?;
        let info: IAsyncInfo = operation.cast().map_err(Error::WindowsError)?;
        let value = WinRTValue::Async(AsyncInfo {
            info,
            async_type: MetadataTable::new().async_action(),
        });
        let (sender, receiver) = std::sync::mpsc::sync_channel(1);

        super::set_async_completed_handler(
            &value,
            Box::new(move || {
                let _ = sender.send(());
            }),
        )?;

        receiver
            .recv_timeout(std::time::Duration::from_secs(5))
            .expect("completion handler should fire");
        assert!(matches!(
            super::get_async_results(&value)?,
            WinRTValue::HResult(windows_core::HRESULT(0))
        ));
        Ok(())
    }

    /// Verify progress handler IID computation matches windows-rs for known types.
    #[test]
    fn test_progress_handler_iid_u64_u64() {
        use crate::metadata_table::TypeKind;

        let reg = MetadataTable::new();
        let t_u64 = reg.make(TypeKind::U64);
        let p_u64 = reg.make(TypeKind::U64);
        let async_type = reg.async_operation_with_progress(&t_u64, &p_u64);

        // Compute our progress handler IID
        let our_iid = async_type
            .progress_handler_iid()
            .expect("should compute progress handler IID");

        // Expected IID from windows-rs:
        // AsyncOperationProgressHandler<u64, u64>
        let expected_iid =
            <windows_future::AsyncOperationProgressHandler<u64, u64> as Interface>::IID;

        assert_eq!(
            our_iid, expected_iid,
            "Progress handler IID mismatch for <u64, u64>: ours={:?} expected={:?}",
            our_iid, expected_iid
        );
        println!("Progress handler IID for <u64, u64>: {:?}", our_iid);
    }

    #[test]
    fn test_progress_handler_marshals_u64_value() {
        use crate::metadata_table::TypeKind;
        use std::sync::{
            Arc,
            atomic::{AtomicU64, Ordering},
        };
        use windows_core::{HRESULT, IUnknown_Vtbl};

        #[repr(C)]
        struct ProgressHandlerVtbl {
            base: IUnknown_Vtbl,
            invoke: unsafe extern "system" fn(
                *mut std::ffi::c_void,
                *mut std::ffi::c_void,
                *mut std::ffi::c_void,
            ) -> HRESULT,
        }

        let reg = MetadataTable::new();
        let result_type = reg.make(TypeKind::U64);
        let progress_type = reg.make(TypeKind::U64);
        let async_type = reg.async_operation_with_progress(&result_type, &progress_type);
        let handler_iid = async_type
            .progress_handler_iid()
            .expect("should have progress handler IID");

        let received = Arc::new(AtomicU64::new(0));
        let received_callback = received.clone();
        let handler = super::create_progress_handler(
            handler_iid,
            progress_type,
            Box::new(move |value| {
                if let WinRTValue::U64(value) = value {
                    received_callback.store(value, Ordering::SeqCst);
                }
            }),
        );

        let vtable = unsafe { &**(handler.as_raw() as *const *const ProgressHandlerVtbl) };
        let result = unsafe {
            (vtable.invoke)(
                handler.as_raw(),
                std::ptr::null_mut(),
                42usize as *mut std::ffi::c_void,
            )
        };

        assert!(result.is_ok());
        assert_eq!(received.load(Ordering::SeqCst), 42);
    }

    /// Test SetProgress on a real IAsyncOperationWithProgress using HTTP BufferAllAsync.
    #[tokio::test]
    async fn test_progress_handler_with_http() -> Result<()> {
        use crate::metadata_table::TypeKind;
        use std::sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        };

        // Create an HTTP client and make a request to get an IAsyncOperationWithProgress<u64, u64>
        let client = windows::Web::Http::HttpClient::new().map_err(Error::WindowsError)?;
        let uri = windows::Foundation::Uri::CreateUri(&windows_core::HSTRING::from(
            "https://httpbin.org/bytes/1024",
        ))
        .map_err(Error::WindowsError)?;

        let response_op = client.GetAsync(&uri).map_err(Error::WindowsError)?;
        let response = response_op.await.map_err(Error::WindowsError)?;
        let content = response.Content().map_err(Error::WindowsError)?;

        // BufferAllAsync returns IAsyncOperationWithProgress<u64, u64>
        let buffer_op = content.BufferAllAsync().map_err(Error::WindowsError)?;

        // Wrap as our dynamic type
        let info: IAsyncInfo = buffer_op.cast().map_err(Error::WindowsError)?;

        let reg = MetadataTable::new();
        let t_u64 = reg.make(TypeKind::U64);
        let p_u64 = reg.make(TypeKind::U64);
        let async_type = reg.async_operation_with_progress(&t_u64, &p_u64);

        let async_info = AsyncInfo {
            info,
            async_type: async_type.clone(),
        };

        // Set up progress handler
        let progress_count = Arc::new(AtomicU32::new(0));
        let progress_count2 = progress_count.clone();

        let progress_type = async_info
            .progress_type()
            .expect("should have progress type");
        let handler_iid = async_info
            .progress_handler_iid()
            .expect("should have handler IID");

        let progress_cb: super::ProgressCallback = Box::new(move |val: WinRTValue| {
            println!("Progress callback fired: {:?}", val);
            progress_count2.fetch_add(1, Ordering::SeqCst);
        });

        let handler = super::create_progress_handler(handler_iid, progress_type, progress_cb);

        // Call SetProgress - this is what was crashing
        async_info
            .set_progress_handler(&handler)
            .expect("SetProgress should succeed");

        println!("SetProgress succeeded! Awaiting result...");

        // Await the result
        let value = WinRTValue::Async(async_info);
        let result = value.await?;
        println!("BufferAllAsync completed: {:?}", result);
        println!(
            "Progress callbacks received: {}",
            progress_count.load(Ordering::SeqCst)
        );

        Ok(())
    }

    /// Verify completed handler IID for IAsyncOperationWithProgress<u64,u64>
    #[test]
    fn test_completed_handler_iid_u64_u64() {
        use crate::metadata_table::TypeKind;

        let reg = MetadataTable::new();
        let t_u64 = reg.make(TypeKind::U64);
        let p_u64 = reg.make(TypeKind::U64);
        let async_type = reg.async_operation_with_progress(&t_u64, &p_u64);

        let our_iid = async_type
            .completed_handler_iid()
            .expect("should compute completed handler IID");

        let expected_iid = <windows_future::AsyncOperationWithProgressCompletedHandler<u64, u64>
            as Interface>::IID;

        assert_eq!(
            our_iid, expected_iid,
            "Completed handler IID mismatch: ours={:?} expected={:?}",
            our_iid, expected_iid
        );
        println!("Completed handler IID for <u64, u64>: {:?}", our_iid);
    }

    /// Verify async interface IID for IAsyncOperationWithProgress<u64,u64>
    #[test]
    fn test_async_iid_u64_u64() {
        use crate::metadata_table::TypeKind;

        let reg = MetadataTable::new();
        let t_u64 = reg.make(TypeKind::U64);
        let p_u64 = reg.make(TypeKind::U64);
        let async_type = reg.async_operation_with_progress(&t_u64, &p_u64);

        let our_iid = async_type.iid().expect("should compute async IID");

        let expected_iid =
            <windows_future::IAsyncOperationWithProgress<u64, u64> as Interface>::IID;

        assert_eq!(
            our_iid, expected_iid,
            "Async IID mismatch: ours={:?} expected={:?}",
            our_iid, expected_iid
        );
        println!(
            "Async IID for IAsyncOperationWithProgress<u64, u64>: {:?}",
            our_iid
        );
    }

    /// Test await on WithProgress WITHOUT setting progress handler (baseline).
    #[tokio::test]
    async fn test_with_progress_no_handler() -> Result<()> {
        use crate::metadata_table::TypeKind;

        let client = windows::Web::Http::HttpClient::new().map_err(Error::WindowsError)?;
        let uri = windows::Foundation::Uri::CreateUri(&windows_core::HSTRING::from(
            "https://httpbin.org/bytes/1024",
        ))
        .map_err(Error::WindowsError)?;

        let response_op = client.GetAsync(&uri).map_err(Error::WindowsError)?;
        let response = response_op.await.map_err(Error::WindowsError)?;
        let content = response.Content().map_err(Error::WindowsError)?;

        let buffer_op = content.BufferAllAsync().map_err(Error::WindowsError)?;

        let info: IAsyncInfo = buffer_op.cast().map_err(Error::WindowsError)?;

        let reg = MetadataTable::new();
        let t_u64 = reg.make(TypeKind::U64);
        let p_u64 = reg.make(TypeKind::U64);
        let async_type = reg.async_operation_with_progress(&t_u64, &p_u64);

        let value = WinRTValue::Async(AsyncInfo { info, async_type });

        let result = value.await?;
        println!("WithProgress (no handler) completed: {:?}", result);
        Ok(())
    }

    /// Verify get_results for IAsyncOperationWithProgress<u32, u32> (WriteAsync).
    /// Compares dynwinrt result against windows-rs typed result.
    #[tokio::test]
    async fn test_get_results_u32_write_async() -> Result<()> {
        use crate::metadata_table::TypeKind;
        use windows::Storage::Streams::{Buffer, IOutputStream, InMemoryRandomAccessStream};

        let stream = InMemoryRandomAccessStream::new().map_err(Error::WindowsError)?;
        let output: IOutputStream = stream.cast().map_err(Error::WindowsError)?;

        let buf_size: u32 = 1234;
        let buffer = Buffer::Create(buf_size).map_err(Error::WindowsError)?;
        buffer.SetLength(buf_size).map_err(Error::WindowsError)?;

        // windows-rs typed call for reference
        let typed_op = output.WriteAsync(&buffer).map_err(Error::WindowsError)?;
        let typed_result = typed_op.await.map_err(Error::WindowsError)?;
        println!("windows-rs WriteAsync result: {} bytes", typed_result);
        assert_eq!(typed_result, buf_size);

        // Now test via dynwinrt
        let buffer2 = Buffer::Create(buf_size).map_err(Error::WindowsError)?;
        buffer2.SetLength(buf_size).map_err(Error::WindowsError)?;

        let dyn_op = output.WriteAsync(&buffer2).map_err(Error::WindowsError)?;
        let info: IAsyncInfo = dyn_op.cast().map_err(Error::WindowsError)?;

        let reg = MetadataTable::new();
        let t_u32 = reg.make(TypeKind::U32);
        let p_u32 = reg.make(TypeKind::U32);
        let async_type = reg.async_operation_with_progress(&t_u32, &p_u32);

        let value = WinRTValue::Async(AsyncInfo { info, async_type });
        let result = value.await?;
        println!("dynwinrt WriteAsync result: {:?}", result);

        match result {
            WinRTValue::U32(v) => assert_eq!(v, buf_size, "u32 result mismatch"),
            other => panic!("Expected U32, got {:?}", other),
        }

        println!("get_results u32 verification passed!");
        Ok(())
    }

    /// Verify get_results for IAsyncOperationWithProgress<u64, u64> (BufferAllAsync).
    /// Compares dynwinrt result against windows-rs typed result.
    #[tokio::test]
    async fn test_get_results_u64_buffer_all() -> Result<()> {
        use crate::metadata_table::TypeKind;
        use windows::Storage::Streams::Buffer;
        use windows::Web::Http::HttpBufferContent;

        fn buffer_content(size: u32) -> Result<HttpBufferContent> {
            let buffer = Buffer::Create(size).map_err(Error::WindowsError)?;
            buffer.SetLength(size).map_err(Error::WindowsError)?;
            HttpBufferContent::CreateFromBuffer(&buffer).map_err(Error::WindowsError)
        }

        let expected_size = 512u64;
        let content = buffer_content(expected_size as u32)?;

        // windows-rs typed
        let typed_result = content
            .BufferAllAsync()
            .map_err(Error::WindowsError)?
            .await
            .map_err(Error::WindowsError)?;
        println!("windows-rs BufferAllAsync: {} bytes", typed_result);
        assert_eq!(typed_result, expected_size);

        // dynwinrt
        let content2 = buffer_content(expected_size as u32)?;
        let dyn_op = content2.BufferAllAsync().map_err(Error::WindowsError)?;
        let info: IAsyncInfo = dyn_op.cast().map_err(Error::WindowsError)?;

        let reg = MetadataTable::new();
        let t_u64 = reg.make(TypeKind::U64);
        let p_u64 = reg.make(TypeKind::U64);
        let async_type = reg.async_operation_with_progress(&t_u64, &p_u64);

        let value = WinRTValue::Async(AsyncInfo { info, async_type });
        let result = value.await?;
        println!("dynwinrt BufferAllAsync: {:?}", result);

        match result {
            WinRTValue::U64(v) => assert_eq!(v, expected_size, "u64 result mismatch"),
            other => panic!("Expected U64, got {:?}", other),
        }

        println!("get_results u64 verification passed!");
        Ok(())
    }

    /// Verify that `AsyncInfo::cancel()` causes the future to resolve with
    /// `Error::Canceled`. Uses ThreadPool.RunAsync with a sleep — the work item
    /// cooperatively checks status, but more importantly we cancel BEFORE first
    /// poll so the fast path in `WinRTAsyncFuture::poll` sees Canceled status.
    #[tokio::test]
    async fn test_async_cancel_returns_canceled() -> Result<()> {
        // Spawn a long-running work item (5s sleep). We cancel before awaiting,
        // so the operation never gets to "Started" → "Completed" — it transitions
        // straight to "Canceled" once the runtime observes the request.
        let handler = WorkItemHandler::new(|_action| {
            std::thread::sleep(std::time::Duration::from_secs(5));
            Ok(())
        });
        let op = ThreadPool::RunAsync(&handler).map_err(Error::WindowsError)?;
        let info: IAsyncInfo = op.cast().map_err(Error::WindowsError)?;

        let reg = MetadataTable::new();
        let async_info = AsyncInfo {
            info,
            async_type: reg.async_action(),
        };

        // Cancel immediately, before any poll.
        async_info.cancel()?;

        // Allow the WinRT runtime a brief moment to flip the status to Canceled.
        // Without this, a race may leave the operation in Started state and the
        // SetCompleted callback path will eventually fire as Canceled instead.
        std::thread::sleep(std::time::Duration::from_millis(100));

        let value = WinRTValue::Async(async_info);
        let result = value.await;

        match result {
            Err(Error::Canceled) => {
                println!("Got Error::Canceled as expected");
                Ok(())
            }
            other => panic!(
                "Expected Err(Error::Canceled), got {:?}",
                other
                    .map(|v| format!("Ok({:?})", v))
                    .unwrap_or_else(|e| format!("Err({:?})", e))
            ),
        }
    }

    #[test]
    fn test_cancel_on_drop_cancels_pending_operation() -> Result<()> {
        use std::sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        };

        let started = Arc::new(AtomicBool::new(false));
        let started_callback = started.clone();
        let release = Arc::new(AtomicBool::new(false));
        let release_callback = release.clone();
        let handler = WorkItemHandler::new(move |_| {
            started_callback.store(true, Ordering::SeqCst);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
            while !release_callback.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
            Ok(())
        });
        let operation = ThreadPool::RunAsync(&handler).map_err(Error::WindowsError)?;
        let info: IAsyncInfo = operation.cast().map_err(Error::WindowsError)?;
        let value = WinRTValue::Async(AsyncInfo {
            info: info.clone(),
            async_type: MetadataTable::new().async_action(),
        });

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while !started.load(Ordering::SeqCst) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(started.load(Ordering::SeqCst), "work item did not start");

        drop(value.into_future().cancel_on_drop());

        let cancel_deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while info.Status().map_err(Error::WindowsError)? == AsyncStatus::Started
            && std::time::Instant::now() < cancel_deadline
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let status = info.Status().map_err(Error::WindowsError)?;
        release.store(true, Ordering::SeqCst);
        assert_eq!(status, AsyncStatus::Canceled);
        Ok(())
    }

    /// Verify `cancel()` on an already-completed async operation is a no-op
    /// (does not error). WinRT spec: Cancel after completion is silently ignored.
    #[tokio::test]
    async fn test_async_cancel_after_completion_is_noop() -> Result<()> {
        let handler = WorkItemHandler::new(|_| Ok(()));
        let op = ThreadPool::RunAsync(&handler).map_err(Error::WindowsError)?;
        let info: IAsyncInfo = op.cast().map_err(Error::WindowsError)?;

        let reg = MetadataTable::new();
        let async_info = AsyncInfo {
            info: info.clone(),
            async_type: reg.async_action(),
        };

        // Wait for completion first.
        let value = WinRTValue::Async(AsyncInfo {
            info,
            async_type: reg.async_action(),
        });
        let _ = value.await?;

        // Cancel after completion — must not error.
        async_info.cancel()?;
        println!("cancel() after completion succeeded (no-op)");
        Ok(())
    }
}
