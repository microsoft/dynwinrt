// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Bounded native file I/O. An owning completion carries its buffer, resource
//! lease and capacity permit until the receiver consumes or drops it.

use core::ffi::c_void;
use std::collections::HashMap;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use windows::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
use windows::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows::Win32::System::IO::{
    CancelIoEx, CreateIoCompletionPort, GetOverlappedResult, GetQueuedCompletionStatus, OVERLAPPED,
    OVERLAPPED_0_0,
};
use windows::Win32::System::WindowsProgramming::FILE_SKIP_COMPLETION_PORT_ON_SUCCESS;

use super::{Cleanup, OwnedResource, OwnedResourceAsyncLease};

pub const COMPLETION_WORKERS_MAX: usize = 4;
pub const MAX_PENDING_OPERATIONS: usize = 1024;
pub const MAX_OPERATION_BUFFER_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_PENDING_BUFFER_BYTES: usize = 256 * 1024 * 1024;

const ERROR_IO_PENDING: u32 = 997;
const ERROR_OPERATION_ABORTED: u32 = 995;
const ERROR_HANDLE_EOF: u32 = 38;
const ERROR_BROKEN_PIPE: u32 = 109;
const ERROR_NOT_FOUND: u32 = 1168;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IoKind {
    Read,
    Write,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Submission {
    Pending,
    CompletionPacket,
    Inline,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IoError {
    message: String,
    code: Option<u32>,
}

impl IoError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
            code: None,
        }
    }

    fn native(function: &str, code: u32) -> Self {
        Self {
            message: format!("{function} failed with Win32 error {code}"),
            code: Some(code),
        }
    }

    pub fn code(&self) -> Option<u32> {
        self.code
    }
}

impl fmt::Display for IoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.message.fmt(formatter)
    }
}

impl std::error::Error for IoError {}

type IoResult<T> = std::result::Result<T, IoError>;

struct NativeControl {
    active: bool,
    handle: usize,
    overlapped: *const OVERLAPPED,
}

struct OperationState {
    control: Mutex<NativeControl>,
    cancelled: AtomicBool,
}

// The pointer is accessed only with `control` locked. It remains in a stable
// registry allocation until deactivation clears it under that same lock.
unsafe impl Send for OperationState {}
unsafe impl Sync for OperationState {}

impl OperationState {
    fn activate(&self, handle: usize, overlapped: *const OVERLAPPED) {
        let mut control = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        control.handle = handle;
        control.overlapped = overlapped;
        control.active = true;
    }

    fn deactivate(&self) {
        let mut control = self
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        control.active = false;
        control.handle = 0;
        control.overlapped = std::ptr::null();
    }
}

#[derive(Clone)]
pub struct IoCancellation(Arc<OperationState>);

impl IoCancellation {
    fn new() -> Self {
        Self(Arc::new(OperationState {
            control: Mutex::new(NativeControl {
                active: false,
                handle: 0,
                overlapped: std::ptr::null(),
            }),
            cancelled: AtomicBool::new(false),
        }))
    }

    /// Requests cancellation; it does not retire OS-owned storage.
    pub fn cancel(&self) {
        self.0.cancelled.store(true, Ordering::Release);
        let control = self
            .0
            .control
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        if control.active {
            if let Err(error) = unsafe {
                CancelIoEx(
                    HANDLE(control.handle as *mut c_void),
                    Some(control.overlapped),
                )
            } {
                if win32_error_code(&error) != ERROR_NOT_FOUND {
                    eprintln!("[dynwinrt] native I/O cancellation request failed: {error}");
                }
            }
        }
    }

    fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(Ordering::Acquire)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CapacityUsage {
    pub operations: usize,
    pub buffer_bytes: usize,
}

struct CapacityPermit {
    capacity: Arc<Mutex<CapacityUsage>>,
    buffer_bytes: usize,
}

impl CapacityPermit {
    fn acquire(capacity: &Arc<Mutex<CapacityUsage>>, buffer_bytes: usize) -> IoResult<Self> {
        let mut state = capacity.lock().unwrap_or_else(|error| error.into_inner());
        validate_capacity(state.operations, state.buffer_bytes, buffer_bytes)?;
        state.operations += 1;
        state.buffer_bytes += buffer_bytes;
        drop(state);
        Ok(Self {
            capacity: Arc::clone(capacity),
            buffer_bytes,
        })
    }
}

impl Drop for CapacityPermit {
    fn drop(&mut self) {
        let mut state = self
            .capacity
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        state.operations = state
            .operations
            .checked_sub(1)
            .expect("IOCP operation accounting remains balanced");
        state.buffer_bytes = state
            .buffer_bytes
            .checked_sub(self.buffer_bytes)
            .expect("IOCP buffer accounting remains balanced");
    }
}

struct IoStorage {
    kind: IoKind,
    lease: OwnedResourceAsyncLease,
    buffer: Vec<u8>,
    _permit: CapacityPermit,
}

/// Prepared work is not yet owned by the OS. Dropping it returns its lease and
/// capacity without submitting anything.
pub struct PreparedIo {
    runtime: Arc<IocpRuntime>,
    storage: IoStorage,
    offset: u64,
    cancellation: IoCancellation,
}

impl PreparedIo {
    pub fn cancellation(&self) -> IoCancellation {
        self.cancellation.clone()
    }

    /// The receiver must not block a completion worker. Dropping or rejecting
    /// its owning record retires the buffer, resource lease and quota together.
    pub fn submit(
        self,
        receiver: impl FnOnce(IoCompletion) + Send + 'static,
    ) -> IoResult<Submission> {
        let runtime = Arc::clone(&self.runtime);
        runtime.submit(self, Box::new(receiver))
    }
}

/// A terminal result, including ownership of all storage still awaiting
/// delivery. Merely receiving a native packet does not return capacity.
pub struct IoCompletion {
    storage: IoStorage,
    result: IoResult<u32>,
}

impl IoCompletion {
    pub fn kind(&self) -> IoKind {
        self.storage.kind
    }

    pub fn result(&self) -> std::result::Result<u32, &IoError> {
        self.result.as_ref().copied()
    }

    pub fn buffer(&self) -> &[u8] {
        &self.storage.buffer
    }
}

type CompletionReceiver = Box<dyn FnOnce(IoCompletion) + Send + 'static>;

struct NativeOperation {
    overlapped: OVERLAPPED,
    storage: IoStorage,
    cancellation: IoCancellation,
    receiver: CompletionReceiver,
}

// The operation is moved only into or out of the mutex-protected registry.
// Windows can access its stable allocation only between activation and the
// terminal packet (or verified synchronous completion without a packet).
unsafe impl Send for NativeOperation {}

impl NativeOperation {
    fn deactivate(&mut self) {
        self.cancellation.0.deactivate();
        self.storage.lease.mark_inactive();
    }

    fn deliver(mut self: Box<Self>, transferred: u32, error: Option<u32>) {
        self.deactivate();
        let result = match error {
            Some(error) if is_read_eof(self.storage.kind, error) => Ok(0),
            Some(error) => Err(IoError::native(
                if error == ERROR_OPERATION_ABORTED {
                    "OVERLAPPED operation"
                } else {
                    "IOCP completion"
                },
                error,
            )),
            None => Ok(transferred),
        };
        let Self {
            storage, receiver, ..
        } = *self;
        let completion = IoCompletion { storage, result };
        // A Rust consumer panic must not permanently reduce the shared worker
        // pool or prevent later native operations from retiring.
        if std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| receiver(completion))).is_err()
        {
            eprintln!("[dynwinrt] native I/O completion receiver panicked");
        }
    }
}

pub struct IocpRuntime {
    port: usize,
    registry: Mutex<HashMap<usize, Box<NativeOperation>>>,
    capacity: Arc<Mutex<CapacityUsage>>,
    shutting_down: AtomicBool,
    worker_count: usize,
}

static RUNTIME: LazyLock<IoResult<Arc<IocpRuntime>>> = LazyLock::new(IocpRuntime::new);

impl IocpRuntime {
    pub fn shared() -> IoResult<&'static Arc<Self>> {
        RUNTIME.as_ref().map_err(Clone::clone)
    }

    fn new() -> IoResult<Arc<Self>> {
        use windows::Win32::Foundation::HMODULE;
        use windows::Win32::System::LibraryLoader::{
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_PIN,
            GetModuleHandleExW,
        };

        // Workers can outlive the embedding environment and any loaded addon.
        let mut module = HMODULE::default();
        unsafe {
            GetModuleHandleExW(
                GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_PIN,
                windows_core::PCWSTR(Self::new as *const () as *const u16),
                &mut module,
            )
        }
        .map_err(|error| IoError::new(format!("Cannot pin the IOCP worker module: {error}")))?;
        let worker_count = std::thread::available_parallelism()
            .map(|count| count.get().min(COMPLETION_WORKERS_MAX))
            .unwrap_or(2)
            .max(1);
        let port =
            unsafe { CreateIoCompletionPort(INVALID_HANDLE_VALUE, None, 0, worker_count as u32) }
                .map_err(|error| IoError::new(format!("CreateIoCompletionPort failed: {error}")))?;
        let runtime = Arc::new(Self {
            port: port.0 as usize,
            registry: Mutex::new(HashMap::new()),
            capacity: Arc::new(Mutex::new(CapacityUsage::default())),
            shutting_down: AtomicBool::new(false),
            worker_count,
        });
        for index in 0..worker_count {
            let worker = Arc::clone(&runtime);
            if let Err(error) = std::thread::Builder::new()
                .name(format!("dynwinrt-iocp-completion-{index}"))
                .spawn(move || worker.completion_loop())
            {
                runtime.shutting_down.store(true, Ordering::Release);
                let _ = unsafe { windows::Win32::Foundation::CloseHandle(port) };
                return Err(IoError::new(format!(
                    "Failed to create IOCP completion worker: {error}"
                )));
            }
        }
        Ok(runtime)
    }

    pub fn worker_count(&self) -> usize {
        self.worker_count
    }

    pub fn capacity_usage(&self) -> CapacityUsage {
        *self
            .capacity
            .lock()
            .unwrap_or_else(|error| error.into_inner())
    }

    pub fn prepare_read(
        self: &Arc<Self>,
        resource: &Arc<OwnedResource>,
        length: usize,
        offset: u64,
    ) -> IoResult<PreparedIo> {
        self.prepare(resource, IoKind::Read, length, &[], offset)
    }

    /// Takes an immediate native snapshot; no caller-owned buffer is retained
    /// or subsequently read by Windows.
    pub fn prepare_write(
        self: &Arc<Self>,
        resource: &Arc<OwnedResource>,
        bytes: &[u8],
        offset: u64,
    ) -> IoResult<PreparedIo> {
        self.prepare(resource, IoKind::Write, bytes.len(), bytes, offset)
    }

    fn prepare(
        self: &Arc<Self>,
        resource: &Arc<OwnedResource>,
        kind: IoKind,
        length: usize,
        bytes: &[u8],
        offset: u64,
    ) -> IoResult<PreparedIo> {
        if resource.cleanup() != Cleanup::CloseHandle {
            return Err(IoError::new(
                "OVERLAPPED I/O requires a CloseHandle resource",
            ));
        }
        validate_operation_buffer(length)?;
        let permit = CapacityPermit::acquire(&self.capacity, length)?;
        let lease = resource
            .async_lease(Cleanup::CloseHandle)
            .map_err(|error| IoError::new(error.message()))?;
        if lease.raw() == usize::MAX {
            return Err(IoError::new("OVERLAPPED I/O requires a valid file HANDLE"));
        }
        let mut buffer = Vec::new();
        buffer
            .try_reserve_exact(length)
            .map_err(|_| IoError::new("Unable to allocate private OVERLAPPED I/O buffer"))?;
        match kind {
            IoKind::Read => buffer.resize(length, 0),
            IoKind::Write => buffer.extend_from_slice(bytes),
        }
        Ok(PreparedIo {
            runtime: Arc::clone(self),
            storage: IoStorage {
                kind,
                lease,
                buffer,
                _permit: permit,
            },
            offset,
            cancellation: IoCancellation::new(),
        })
    }

    fn port(&self) -> HANDLE {
        HANDLE(self.port as *mut c_void)
    }

    fn submit(&self, task: PreparedIo, receiver: CompletionReceiver) -> IoResult<Submission> {
        if task.cancellation.is_cancelled() {
            return Err(IoError::new("OVERLAPPED operation was aborted"));
        }
        let handle = task.storage.lease.raw();
        let mut overlapped = OVERLAPPED::default();
        overlapped.Anonymous.Anonymous = OVERLAPPED_0_0 {
            Offset: task.offset as u32,
            OffsetHigh: (task.offset >> 32) as u32,
        };
        let mut operation = Box::new(NativeOperation {
            overlapped,
            storage: task.storage,
            cancellation: task.cancellation,
            receiver,
        });
        let overlapped_ptr = &mut operation.overlapped as *mut OVERLAPPED;
        let key = overlapped_ptr as usize;
        let mut registry = self
            .registry
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        operation
            .storage
            .lease
            .associate_completion_port(self.port)
            .map_err(|error| IoError::new(error.message()))?;
        let completion_modes = operation
            .storage
            .lease
            .completion_modes()
            .map_err(|error| IoError::new(error.message()))?;
        if registry.contains_key(&key) {
            return Err(IoError::new("duplicate OVERLAPPED operation address"));
        }
        operation.cancellation.0.activate(handle, overlapped_ptr);
        operation.storage.lease.mark_active();
        registry.insert(key, operation);
        let operation = registry
            .get_mut(&key)
            .expect("operation was just registered");
        let result = unsafe {
            match operation.storage.kind {
                IoKind::Read => ReadFile(
                    HANDLE(handle as *mut c_void),
                    Some(operation.storage.buffer.as_mut_slice()),
                    None,
                    Some(overlapped_ptr),
                ),
                IoKind::Write => WriteFile(
                    HANDLE(handle as *mut c_void),
                    Some(operation.storage.buffer.as_slice()),
                    None,
                    Some(overlapped_ptr),
                ),
            }
        };
        let error = result.err().map(|error| win32_error_code(&error));
        if let Some(error) = error.filter(|error| *error != ERROR_IO_PENDING) {
            let mut operation = registry
                .remove(&key)
                .expect("failed operation was registered");
            drop(registry);
            if is_read_eof(operation.storage.kind, error) {
                operation.deliver(0, None);
                return Ok(Submission::Inline);
            }
            operation.deactivate();
            return Err(IoError::native(
                match operation.storage.kind {
                    IoKind::Read => "ReadFile",
                    IoKind::Write => "WriteFile",
                },
                error,
            ));
        }
        if error.is_none() && completion_modes & FILE_SKIP_COMPLETION_PORT_ON_SUCCESS != 0 {
            let mut transferred = 0;
            let result = unsafe {
                GetOverlappedResult(
                    HANDLE(handle as *mut c_void),
                    overlapped_ptr,
                    &mut transferred,
                    false,
                )
            };
            let error = result.err().map(|error| win32_error_code(&error));
            let operation = registry
                .remove(&key)
                .expect("inline operation was registered");
            drop(registry);
            operation.deliver(transferred, error);
            return Ok(Submission::Inline);
        }
        let operation = registry
            .get(&key)
            .expect("submitted operation remains registered");
        if operation.cancellation.is_cancelled() {
            // A request may have raced activation before ReadFile/WriteFile
            // actually submitted the operation. Retry once after submission.
            operation.cancellation.cancel();
        }
        Ok(if error.is_some() {
            Submission::Pending
        } else {
            Submission::CompletionPacket
        })
    }

    fn completion_loop(self: &Arc<Self>) {
        loop {
            let mut transferred = 0;
            let mut completion_key = 0;
            let mut overlapped = std::ptr::null_mut();
            let result = unsafe {
                GetQueuedCompletionStatus(
                    self.port(),
                    &mut transferred,
                    &mut completion_key,
                    &mut overlapped,
                    u32::MAX,
                )
            };
            if overlapped.is_null() {
                if self.shutting_down.load(Ordering::Acquire) {
                    return;
                }
                if let Err(error) = result {
                    eprintln!("[dynwinrt] IOCP completion wait failed: {error}");
                }
                continue;
            }
            let operation = self
                .registry
                .lock()
                .unwrap_or_else(|error| error.into_inner())
                .remove(&(overlapped as usize));
            if let Some(operation) = operation {
                operation.deliver(
                    transferred,
                    result.err().map(|error| win32_error_code(&error)),
                );
            } else {
                eprintln!("[dynwinrt] ignored completion for unknown OVERLAPPED {overlapped:p}");
            }
        }
    }
}

fn is_read_eof(kind: IoKind, error: u32) -> bool {
    kind == IoKind::Read && matches!(error, ERROR_HANDLE_EOF | ERROR_BROKEN_PIPE)
}

fn win32_error_code(error: &windows_core::Error) -> u32 {
    let code = error.code().0 as u32;
    if code & 0xffff_0000 == 0x8007_0000 {
        code & 0xffff
    } else {
        code
    }
}

fn validate_operation_buffer(bytes: usize) -> IoResult<()> {
    if bytes > MAX_OPERATION_BUFFER_BYTES {
        return Err(IoError::new(format!(
            "IOCP operation Buffer exceeds the {MAX_OPERATION_BUFFER_BYTES} byte limit"
        )));
    }
    Ok(())
}

fn validate_capacity(operations: usize, bytes: usize, requested: usize) -> IoResult<()> {
    validate_operation_buffer(requested)?;
    if operations >= MAX_PENDING_OPERATIONS {
        return Err(IoError::new(format!(
            "IOCP pending operation limit ({MAX_PENDING_OPERATIONS}) was reached"
        )));
    }
    let total = bytes
        .checked_add(requested)
        .ok_or_else(|| IoError::new("IOCP pending Buffer accounting overflow"))?;
    if total > MAX_PENDING_BUFFER_BYTES {
        return Err(IoError::new(format!(
            "IOCP pending native Buffer limit ({MAX_PENDING_BUFFER_BYTES} bytes) would be exceeded"
        )));
    }
    Ok(())
}

#[cfg(test)]
#[path = "win32_io_tests.rs"]
mod tests;
