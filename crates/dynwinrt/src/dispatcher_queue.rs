// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use windows::Foundation::TypedEventHandler;
use windows::System::{
    DispatcherQueue, DispatcherQueueController, DispatcherQueueHandler,
    DispatcherQueueShutdownStartingEventArgs,
};
use windows::Win32::System::WinRT::{
    CreateDispatcherQueueController, DQTAT_COM_STA, DQTYPE_THREAD_CURRENT, DispatcherQueueOptions,
};

/// A cloneable handle that can enqueue work from any thread onto a captured
/// Windows system dispatcher queue.
#[derive(Clone)]
pub struct SystemDispatcherQueueHandle {
    queue: DispatcherQueue,
}

impl SystemDispatcherQueueHandle {
    pub fn try_enqueue<F>(&self, callback: F) -> windows_core::Result<bool>
    where
        F: Fn() -> windows_core::Result<()> + Send + 'static,
    {
        self.queue
            .TryEnqueue(&DispatcherQueueHandler::new(callback))
    }
}

/// Captures the current thread's `Windows.System.DispatcherQueue`, creating a
/// current-thread controller only when the thread does not already have one.
pub struct SystemDispatcherQueue {
    queue: DispatcherQueue,
    controller: Option<DispatcherQueueController>,
    shutdown_action: Option<windows_future::IAsyncAction>,
    shutdown_starting_token: Option<i64>,
    shutdown_completed_token: Option<i64>,
}

impl SystemDispatcherQueue {
    pub fn ensure_for_current_thread() -> windows_core::Result<Self> {
        match DispatcherQueue::GetForCurrentThread() {
            Ok(queue) => return Ok(Self::new(queue, None)),
            // A successful ABI call with a null queue becomes Error::empty().
            Err(error) if error.code().is_ok() => {}
            Err(error) => return Err(error),
        }

        let options = DispatcherQueueOptions {
            dwSize: std::mem::size_of::<DispatcherQueueOptions>() as u32,
            threadType: DQTYPE_THREAD_CURRENT,
            apartmentType: DQTAT_COM_STA,
        };
        let controller = unsafe { CreateDispatcherQueueController(options)? };
        let queue = controller.DispatcherQueue()?;
        Ok(Self::new(queue, Some(controller)))
    }

    fn new(queue: DispatcherQueue, controller: Option<DispatcherQueueController>) -> Self {
        Self {
            queue,
            controller,
            shutdown_action: None,
            shutdown_starting_token: None,
            shutdown_completed_token: None,
        }
    }

    pub fn was_created(&self) -> bool {
        self.controller.is_some()
    }

    pub fn has_thread_access(&self) -> windows_core::Result<bool> {
        self.queue.HasThreadAccess()
    }

    pub fn handle(&self) -> SystemDispatcherQueueHandle {
        SystemDispatcherQueueHandle {
            queue: self.queue.clone(),
        }
    }

    pub fn observe_shutdown<F, G>(
        &mut self,
        on_starting: F,
        on_completed: G,
    ) -> windows_core::Result<()>
    where
        F: Fn() + Send + 'static,
        G: Fn() + Send + 'static,
    {
        self.remove_shutdown_observers();

        let starting_token = self.queue.ShutdownStarting(&TypedEventHandler::<
            DispatcherQueue,
            DispatcherQueueShutdownStartingEventArgs,
        >::new(move |_, _| {
            on_starting();
            Ok(())
        }))?;
        match self.queue.ShutdownCompleted(&TypedEventHandler::<
            DispatcherQueue,
            windows_core::IInspectable,
        >::new(move |_, _| {
            on_completed();
            Ok(())
        })) {
            Ok(completed_token) => {
                self.shutdown_starting_token = Some(starting_token);
                self.shutdown_completed_token = Some(completed_token);
                Ok(())
            }
            Err(error) => {
                let _ = self.queue.RemoveShutdownStarting(starting_token);
                Err(error)
            }
        }
    }

    pub fn request_shutdown(&mut self) -> windows_core::Result<bool> {
        if self.shutdown_action.is_some() {
            return Ok(false);
        }
        let Some(controller) = self.controller.as_ref() else {
            return Ok(false);
        };

        self.shutdown_action = Some(controller.ShutdownQueueAsync()?);
        Ok(true)
    }

    fn remove_shutdown_observers(&mut self) {
        if let Some(token) = self.shutdown_starting_token.take() {
            let _ = self.queue.RemoveShutdownStarting(token);
        }
        if let Some(token) = self.shutdown_completed_token.take() {
            let _ = self.queue.RemoveShutdownCompleted(token);
        }
    }
}

impl Drop for SystemDispatcherQueue {
    fn drop(&mut self) {
        self.remove_shutdown_observers();
    }
}
