// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use super::*;
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::IntoRawHandle;
use std::path::PathBuf;
use std::sync::atomic::AtomicUsize;
use std::sync::mpsc::{Receiver, channel};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    PIPE_ACCESS_DUPLEX, SetFileCompletionNotificationModes,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_WAIT,
};
use windows_core::HSTRING;

use super::super::ResourceAccess;

static TEST_LOCK: Mutex<()> = Mutex::new(());
static NEXT_NAME: AtomicUsize = AtomicUsize::new(1);
const TIMEOUT: Duration = Duration::from_secs(10);

struct LocalFile {
    resource: Arc<OwnedResource>,
    path: PathBuf,
}

impl LocalFile {
    fn new(write: bool) -> Self {
        let path = PathBuf::from(format!(
            "dynwinrt-native-iocp-{}-{}.bin",
            std::process::id(),
            NEXT_NAME.fetch_add(1, Ordering::Relaxed),
        ));
        std::fs::write(&path, b"native-iocp").unwrap();
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(write)
            .custom_flags(FILE_FLAG_OVERLAPPED.0)
            .open(&path)
            .unwrap();
        let resource =
            unsafe { OwnedResource::adopt(file.into_raw_handle() as usize, Cleanup::CloseHandle) }
                .unwrap();
        Self { resource, path }
    }
}

impl Drop for LocalFile {
    fn drop(&mut self) {
        let _ = self.resource.close();
        let _ = std::fs::remove_file(&self.path);
    }
}

struct Pipe {
    client: Arc<OwnedResource>,
    peer: Arc<OwnedResource>,
}

impl Pipe {
    fn new(modes: u8) -> Self {
        let name = HSTRING::from(format!(
            "\\\\.\\pipe\\dynwinrt-native-iocp-{}-{}",
            std::process::id(),
            NEXT_NAME.fetch_add(1, Ordering::Relaxed),
        ));
        let peer = unsafe {
            CreateNamedPipeW(
                &name,
                PIPE_ACCESS_DUPLEX,
                PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS,
                1,
                4096,
                4096,
                0,
                None,
            )
        };
        assert!(!peer.is_invalid(), "CreateNamedPipeW failed");
        let peer = unsafe { OwnedResource::adopt(peer.0 as usize, Cleanup::CloseHandle) }.unwrap();
        let client = unsafe {
            CreateFileW(
                &name,
                GENERIC_READ.0 | GENERIC_WRITE.0,
                FILE_SHARE_READ | FILE_SHARE_WRITE,
                None,
                OPEN_EXISTING,
                FILE_FLAG_OVERLAPPED,
                None,
            )
        }
        .unwrap();
        let client =
            unsafe { OwnedResource::adopt(client.0 as usize, Cleanup::CloseHandle) }.unwrap();
        let connected = unsafe { ConnectNamedPipe(HANDLE(peer.raw() as *mut c_void), None) };
        assert!(
            connected.is_ok()
                || connected
                    .as_ref()
                    .is_err_and(|error| win32_error_code(error) == 535),
            "ConnectNamedPipe failed: {connected:?}"
        );
        // Deliberately bypass the capability cache: submission must query the
        // native file state, including changes made outside a tracked call.
        unsafe { SetFileCompletionNotificationModes(HANDLE(client.raw() as *mut c_void), modes) }
            .unwrap();
        unsafe { SetFileCompletionNotificationModes(HANDLE(client.raw() as *mut c_void), 0) }
            .unwrap();
        Self { client, peer }
    }

    fn write(&self, bytes: &[u8]) {
        let lease = self.peer.lease(Cleanup::CloseHandle).unwrap();
        let mut written = 0;
        unsafe {
            WriteFile(
                HANDLE(lease.raw() as *mut c_void),
                Some(bytes),
                Some(&mut written),
                None,
            )
        }
        .unwrap();
        assert_eq!(written as usize, bytes.len());
    }

    fn read(&self, bytes: &mut [u8]) {
        let lease = self.peer.lease(Cleanup::CloseHandle).unwrap();
        let mut read = 0;
        unsafe {
            ReadFile(
                HANDLE(lease.raw() as *mut c_void),
                Some(bytes),
                Some(&mut read),
                None,
            )
        }
        .unwrap();
        assert_eq!(read as usize, bytes.len());
    }
}

struct CancelOnDrop(IoCancellation);

impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

fn submit(task: PreparedIo) -> (Submission, Receiver<IoCompletion>, CancelOnDrop) {
    let cancellation = CancelOnDrop(task.cancellation());
    let (sender, receiver) = channel();
    let submission = task
        .submit(move |completion| {
            let _ = sender.send(completion);
        })
        .unwrap();
    (submission, receiver, cancellation)
}

fn receive(receiver: &Receiver<IoCompletion>) -> IoCompletion {
    receiver
        .recv_timeout(TIMEOUT)
        .expect("native I/O did not complete")
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let deadline = Instant::now() + TIMEOUT;
    while !condition() {
        assert!(Instant::now() < deadline, "native I/O state did not settle");
        std::thread::yield_now();
    }
}

fn assert_idle(runtime: &IocpRuntime, resource: &OwnedResource) {
    assert!(!resource.has_async_leases());
    assert!(!resource.has_active_async_io());
    assert_eq!(runtime.capacity_usage(), CapacityUsage::default());
    assert!(runtime.registry.lock().unwrap().is_empty());
}

#[test]
fn native_io_types_are_send_without_a_language_runtime() {
    fn send<T: Send>() {}
    fn send_sync<T: Send + Sync>() {}
    send::<PreparedIo>();
    send::<IoCompletion>();
    send_sync::<IoCancellation>();
    send_sync::<IocpRuntime>();
    send_sync::<OwnedResource>();
}

#[test]
fn capacity_bounds_operations_and_native_buffers() {
    validate_capacity(MAX_PENDING_OPERATIONS - 1, 0, 1).unwrap();
    assert!(
        validate_capacity(MAX_PENDING_OPERATIONS, 0, 1)
            .unwrap_err()
            .to_string()
            .contains("operation limit")
    );
    validate_capacity(0, MAX_PENDING_BUFFER_BYTES - 1, 1).unwrap();
    assert!(
        validate_capacity(0, MAX_PENDING_BUFFER_BYTES, 1)
            .unwrap_err()
            .to_string()
            .contains("Buffer limit")
    );
    assert!(
        validate_operation_buffer(MAX_OPERATION_BUFFER_BYTES + 1)
            .unwrap_err()
            .to_string()
            .contains("operation Buffer")
    );
    assert!(
        validate_capacity(0, usize::MAX, 1)
            .unwrap_err()
            .to_string()
            .contains("accounting overflow")
    );
    let capacity = Arc::new(Mutex::new(CapacityUsage::default()));
    let permit = CapacityPermit::acquire(&capacity, 16).unwrap();
    assert_eq!(
        *capacity.lock().unwrap(),
        CapacityUsage {
            operations: 1,
            buffer_bytes: 16,
        }
    );
    drop(permit);
    assert_eq!(*capacity.lock().unwrap(), CapacityUsage::default());
    let error = windows_core::Error::from_hresult(windows_core::HRESULT(
        0x8007_0000u32.wrapping_add(ERROR_IO_PENDING) as i32,
    ));
    assert_eq!(win32_error_code(&error), ERROR_IO_PENDING);
}

fn capacity_state(capacity: &Mutex<CapacityUsage>) -> (usize, usize) {
    let state = capacity.lock().unwrap();
    (state.operations, state.buffer_bytes)
}

#[test]
fn byte_capacity_and_operation_capacity_are_independent_and_failure_is_transactional() {
    let base = MAX_PENDING_BUFFER_BYTES - MAX_OPERATION_BUFFER_BYTES;
    let capacity = Arc::new(Mutex::new(CapacityUsage {
        operations: 0,
        buffer_bytes: base,
    }));
    let permit = CapacityPermit::acquire(&capacity, MAX_OPERATION_BUFFER_BYTES).unwrap();
    assert_eq!(capacity_state(&capacity), (1, MAX_PENDING_BUFFER_BYTES));
    assert!(CapacityPermit::acquire(&capacity, 1).is_err());
    assert_eq!(capacity_state(&capacity), (1, MAX_PENDING_BUFFER_BYTES));
    let zero = CapacityPermit::acquire(&capacity, 0).unwrap();
    assert_eq!(capacity_state(&capacity), (2, MAX_PENDING_BUFFER_BYTES));
    drop((zero, permit));
    assert_eq!(capacity_state(&capacity), (0, base));
    assert!(CapacityPermit::acquire(&capacity, MAX_OPERATION_BUFFER_BYTES + 1).is_err());
    assert_eq!(capacity_state(&capacity), (0, base));
    let failure = (|| -> IoResult<()> {
        let _permit = CapacityPermit::acquire(&capacity, 16)?;
        Err(IoError::new("operation preparation failed"))
    })();
    assert!(failure.is_err());
    assert_eq!(capacity_state(&capacity), (0, base));
    let panicked = std::panic::catch_unwind(|| {
        let _permit = CapacityPermit::acquire(&capacity, 16).unwrap();
        panic!("operation preparation unwound");
    });
    assert!(panicked.is_err());
    assert_eq!(capacity_state(&capacity), (0, base));
}

#[test]
fn racing_capacity_reservations_admit_only_one_operation_without_timing_assumptions() {
    use std::sync::Barrier;
    const WORKERS: usize = 4;
    let base = CapacityUsage {
        operations: MAX_PENDING_OPERATIONS - 1,
        buffer_bytes: MAX_PENDING_BUFFER_BYTES - 8,
    };
    let capacity = Arc::new(Mutex::new(base));
    let gate = Arc::new(Barrier::new(WORKERS + 1));
    let (result_tx, result_rx) = channel();
    let mut releases = Vec::new();
    let mut workers = Vec::new();
    for _ in 0..WORKERS {
        let capacity = Arc::clone(&capacity);
        let gate = Arc::clone(&gate);
        let result_tx = result_tx.clone();
        let (release_tx, release_rx) = channel();
        releases.push(release_tx);
        workers.push(std::thread::spawn(move || {
            gate.wait();
            let permit = CapacityPermit::acquire(&capacity, 8);
            result_tx.send(permit.is_ok()).unwrap();
            let _ = release_rx.recv();
            drop(permit);
        }));
    }
    gate.wait();
    let outcomes = (0..WORKERS)
        .map(|_| result_rx.recv_timeout(TIMEOUT))
        .collect::<Vec<_>>();
    let held = capacity_state(&capacity);
    for release in releases {
        let _ = release.send(());
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(
        outcomes
            .into_iter()
            .map(Result::unwrap)
            .filter(|admitted| *admitted)
            .count(),
        1
    );
    assert_eq!(held, (MAX_PENDING_OPERATIONS, MAX_PENDING_BUFFER_BYTES));
    assert_eq!(
        capacity_state(&capacity),
        (base.operations, base.buffer_bytes)
    );
}

#[test]
fn completion_deactivation_removes_addresses_before_late_cancellation() {
    let state = IoCancellation::new();
    let event = unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
        .unwrap();
    let event = unsafe { OwnedResource::adopt(event.0 as usize, Cleanup::CloseHandle) }.unwrap();
    let overlapped = Box::new(OVERLAPPED::default());
    state.0.activate(event.raw(), &*overlapped);
    {
        let control = state.0.control.lock().unwrap();
        assert!(control.active);
        assert_eq!(control.overlapped, std::ptr::from_ref(&*overlapped));
    }
    state.0.deactivate();
    drop(overlapped);
    state.cancel();
    state.cancel();
    let control = state.0.control.lock().unwrap();
    assert!(!control.active);
    assert_eq!(control.handle, 0);
    assert!(control.overlapped.is_null());
    assert!(state.is_cancelled());
}

#[test]
fn native_modes_preserve_inline_packets_pending_cancellation_and_eof() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let runtime = IocpRuntime::shared().unwrap();
    assert!((1..=COMPLETION_WORKERS_MAX).contains(&runtime.worker_count()));
    for modes in [0, 1, 2, 3] {
        let pipe = Pipe::new(modes);
        assert_eq!(
            pipe.client.file_capability().cached_completion_modes(),
            None
        );
        pipe.write(b"ready");
        let (submission, receiver, cancel) =
            submit(runtime.prepare_read(&pipe.client, 5, 0).unwrap());
        assert_eq!(
            submission,
            if modes & 1 == 0 {
                Submission::CompletionPacket
            } else {
                Submission::Inline
            }
        );
        let completed = receive(&receiver);
        assert_eq!(completed.result().unwrap(), 5);
        assert_eq!(completed.buffer(), b"ready");
        assert_eq!(
            pipe.client.file_capability().cached_completion_modes(),
            Some(u32::from(modes))
        );
        assert!(!pipe.client.has_active_async_io());
        assert!(pipe.client.has_async_leases());
        assert!(pipe.client.close().is_err());
        assert!(
            pipe.client
                .lock_for_call(ResourceAccess::MutateState("file completion modes"))
                .is_err()
        );
        assert_eq!(runtime.capacity_usage().buffer_bytes, 5);
        drop(completed);
        cancel.0.cancel();
        assert_idle(runtime, &pipe.client);

        let (submission, receiver, cancel) =
            submit(runtime.prepare_read(&pipe.client, 4, 0).unwrap());
        assert_eq!(submission, Submission::Pending);
        {
            // Hold registry retirement, not cancellation. Even after CancelIoEx
            // returns, the OVERLAPPED, native bytes and lease remain owned.
            let registry = runtime.registry.lock().unwrap();
            let operation = registry.values().next().unwrap();
            let pointer = operation.storage.buffer.as_ptr();
            cancel.0.cancel();
            cancel.0.cancel();
            assert!(pipe.client.has_active_async_io());
            assert!(pipe.client.has_async_leases());
            assert_eq!(operation.storage.buffer.as_ptr(), pointer);
            assert_eq!(operation.storage.buffer.len(), 4);
            assert_eq!(runtime.capacity_usage().operations, 1);
            assert!(pipe.client.close().is_err());
        }
        let completed = receive(&receiver);
        assert_eq!(
            completed.result().unwrap_err().code(),
            Some(ERROR_OPERATION_ABORTED)
        );
        assert!(!pipe.client.has_active_async_io());
        assert!(pipe.client.has_async_leases());
        assert_eq!(runtime.capacity_usage().buffer_bytes, 4);
        drop(completed);
        assert_idle(runtime, &pipe.client);

        let (_, receiver, _cancel) = submit(runtime.prepare_read(&pipe.client, 4, 0).unwrap());
        pipe.write(b"next");
        let completed = receive(&receiver);
        assert_eq!(completed.result().unwrap(), 4);
        assert_eq!(completed.buffer(), b"next");
        drop(completed);

        let mut original = Vec::from(b"back");
        let write = runtime.prepare_write(&pipe.client, &original, 0).unwrap();
        original.fill(0);
        let (_, receiver, _cancel) = submit(write);
        let completed = receive(&receiver);
        assert_eq!(completed.kind(), IoKind::Write);
        assert_eq!(completed.result().unwrap(), 4);
        let mut received = [0; 4];
        pipe.read(&mut received);
        assert_eq!(&received, b"back");
        drop(completed);

        pipe.peer.close().unwrap();
        let (_, receiver, _cancel) = submit(runtime.prepare_read(&pipe.client, 4, 0).unwrap());
        let completed = receive(&receiver);
        assert_eq!(completed.result().unwrap(), 0);
        drop(completed);
        assert_idle(runtime, &pipe.client);
        pipe.client.close().unwrap();
    }
}

#[test]
fn prepared_cancellation_submission_errors_and_file_offsets_retire_once() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let runtime = IocpRuntime::shared().unwrap();
    let file = LocalFile::new(false);
    let prepared = runtime.prepare_read(&file.resource, 16, 0).unwrap();
    assert!(file.resource.has_async_leases());
    drop(prepared);
    assert_idle(runtime, &file.resource);

    let prepared = runtime.prepare_read(&file.resource, 16, 0).unwrap();
    prepared.cancellation().cancel();
    let notified = Arc::new(AtomicBool::new(false));
    let received = Arc::clone(&notified);
    let error = prepared
        .submit(move |_| received.store(true, Ordering::Release))
        .unwrap_err();
    assert!(error.to_string().contains("aborted"));
    assert!(!notified.load(Ordering::Acquire));
    assert_idle(runtime, &file.resource);

    let write = runtime.prepare_write(&file.resource, b"denied", 0).unwrap();
    let error = write
        .submit(|_| panic!("failed submission notified"))
        .unwrap_err();
    assert!(error.to_string().contains("WriteFile"));
    assert_idle(runtime, &file.resource);

    let event = unsafe { windows::Win32::System::Threading::CreateEventW(None, true, false, None) }
        .unwrap();
    let event = unsafe { OwnedResource::adopt(event.0 as usize, Cleanup::CloseHandle) }.unwrap();
    let invalid = runtime.prepare_read(&event, 1, 0).unwrap();
    let error = invalid
        .submit(|_| panic!("invalid file notified"))
        .unwrap_err();
    assert!(error.to_string().contains("associate"));
    assert_idle(runtime, &event);
    event.close().unwrap();

    let (_, receiver, _cancel) = submit(runtime.prepare_read(&file.resource, 12, 7).unwrap());
    let completed = receive(&receiver);
    assert_eq!(completed.result().unwrap(), 4);
    assert_eq!(&completed.buffer()[..4], b"iocp");
    assert!(completed.buffer()[4..].iter().all(|byte| *byte == 0));
    drop(completed);
    let (_, receiver, _cancel) = submit(runtime.prepare_read(&file.resource, 4, 1 << 32).unwrap());
    let completed = receive(&receiver);
    assert_eq!(completed.result().unwrap(), 0);
    drop(completed);
    assert_idle(runtime, &file.resource);
}

#[test]
fn dropped_receivers_and_rejected_notifications_keep_pending_storage_until_terminal() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let runtime = IocpRuntime::shared().unwrap();
    let pipe = Pipe::new(0);
    let prepared = runtime.prepare_read(&pipe.client, 4, 0).unwrap();
    let cancellation = CancelOnDrop(prepared.cancellation());
    let (sender, receiver) = channel();
    let (retired, retirement) = channel();
    assert_eq!(
        prepared
            .submit(move |completed| {
                let _ = sender.send(completed);
                retired.send(()).unwrap();
            })
            .unwrap(),
        Submission::Pending
    );
    drop(receiver);
    {
        let registry = runtime.registry.lock().unwrap();
        cancellation.0.cancel();
        assert_eq!(registry.len(), 1);
        assert!(pipe.client.has_async_leases());
        assert!(pipe.client.has_active_async_io());
        assert_eq!(runtime.capacity_usage().buffer_bytes, 4);
    }
    retirement.recv_timeout(TIMEOUT).unwrap();
    assert_idle(runtime, &pipe.client);

    pipe.write(b"drop");
    let (retired, retirement) = channel();
    runtime
        .prepare_read(&pipe.client, 4, 0)
        .unwrap()
        .submit(move |completed| {
            assert_eq!(completed.result().unwrap(), 4);
            drop(completed);
            retired.send(()).unwrap();
        })
        .unwrap();
    retirement.recv_timeout(TIMEOUT).unwrap();
    assert_idle(runtime, &pipe.client);

    let (_, receiver, cancellation) = submit(runtime.prepare_read(&pipe.client, 4, 0).unwrap());
    let weak = Arc::downgrade(&pipe.client);
    drop(pipe.client);
    assert!(weak.upgrade().is_some());
    cancellation.0.cancel();
    let completed = receive(&receiver);
    assert_eq!(
        completed.result().unwrap_err().code(),
        Some(ERROR_OPERATION_ABORTED)
    );
    assert!(weak.upgrade().is_some());
    drop(completed);
    assert!(weak.upgrade().is_none());
    assert_eq!(runtime.capacity_usage(), CapacityUsage::default());
}

#[test]
fn cancellation_racing_success_has_one_terminal_delivery() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let runtime = IocpRuntime::shared().unwrap();
    for modes in [0, 1, 2, 3] {
        for _ in 0..4 {
            let pipe = Pipe::new(modes);
            let (_, receiver, cancellation) =
                submit(runtime.prepare_read(&pipe.client, 4, 0).unwrap());
            std::thread::scope(|scope| {
                scope.spawn(|| pipe.write(b"race"));
                cancellation.0.cancel();
            });
            let completed = receive(&receiver);
            match completed.result() {
                Ok(bytes) => {
                    assert_eq!(bytes, 4);
                    assert_eq!(completed.buffer(), b"race");
                }
                Err(error) => assert_eq!(error.code(), Some(ERROR_OPERATION_ABORTED)),
            }
            assert!(pipe.client.has_async_leases());
            assert!(!pipe.client.has_active_async_io());
            drop(completed);
            cancellation.0.cancel();
            assert!(receiver.try_recv().is_err());
            assert_idle(runtime, &pipe.client);
        }
    }
}

#[test]
fn queued_completions_hold_operation_and_native_byte_quotas_until_consumed() {
    let _serial = TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());
    let runtime = IocpRuntime::shared().unwrap();
    let file = LocalFile::new(false);
    let (sender, receiver) = channel();
    let delivered = Arc::new(AtomicUsize::new(0));
    for _ in 0..MAX_PENDING_OPERATIONS {
        let sender = sender.clone();
        let delivered = Arc::clone(&delivered);
        runtime
            .prepare_read(&file.resource, 1, 0)
            .unwrap()
            .submit(move |completed| {
                let _ = sender.send(completed);
                delivered.fetch_add(1, Ordering::Release);
            })
            .unwrap();
    }
    wait_for(|| delivered.load(Ordering::Acquire) == MAX_PENDING_OPERATIONS);
    assert!(!file.resource.has_active_async_io());
    assert_eq!(
        runtime.capacity_usage(),
        CapacityUsage {
            operations: MAX_PENDING_OPERATIONS,
            buffer_bytes: MAX_PENDING_OPERATIONS,
        }
    );
    let error = runtime.prepare_read(&file.resource, 0, 0).err().unwrap();
    assert!(error.to_string().contains("operation limit"));
    assert!(file.resource.close().is_err());
    drop(receiver);
    assert_idle(runtime, &file.resource);

    let (sender, receiver) = channel();
    let delivered = Arc::new(AtomicUsize::new(0));
    for _ in 0..MAX_PENDING_BUFFER_BYTES / MAX_OPERATION_BUFFER_BYTES {
        let sender = sender.clone();
        let delivered = Arc::clone(&delivered);
        runtime
            .prepare_read(&file.resource, MAX_OPERATION_BUFFER_BYTES, 0)
            .unwrap()
            .submit(move |completed| {
                let _ = sender.send(completed);
                delivered.fetch_add(1, Ordering::Release);
            })
            .unwrap();
    }
    wait_for(|| delivered.load(Ordering::Acquire) == 4);
    assert_eq!(
        runtime.capacity_usage().buffer_bytes,
        MAX_PENDING_BUFFER_BYTES
    );
    let error = runtime.prepare_read(&file.resource, 1, 0).err().unwrap();
    assert!(error.to_string().contains("Buffer limit"));
    let error = runtime
        .prepare_read(&file.resource, MAX_OPERATION_BUFFER_BYTES + 1, 0)
        .err()
        .unwrap();
    assert!(error.to_string().contains("operation Buffer"));
    let completed = receive(&receiver);
    assert_eq!(completed.result().unwrap(), 11);
    assert_eq!(&completed.buffer()[..11], b"native-iocp");
    assert_eq!(
        runtime.capacity_usage().buffer_bytes,
        MAX_PENDING_BUFFER_BYTES
    );
    drop(completed);
    assert_eq!(
        runtime.capacity_usage().buffer_bytes,
        3 * MAX_OPERATION_BUFFER_BYTES
    );
    drop(runtime.prepare_read(&file.resource, 1, 0).unwrap());
    drop(receiver);
    assert_idle(runtime, &file.resource);
}
