// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::sync::Arc;

use super::{Cleanup, Delivery, OwnedResource, ResultPolicy, ResultTarget, Value};
use crate::{
    abi::AbiValue,
    result::{Error, Result},
};

/// A defined owned result whose native cleanup failed.
#[derive(Debug)]
pub struct CleanupFailure {
    target: ResultTarget,
    resource: Arc<OwnedResource>,
    error: windows_core::Error,
}

impl CleanupFailure {
    pub fn target(&self) -> ResultTarget {
        self.target
    }

    pub fn resource(&self) -> &Arc<OwnedResource> {
        &self.resource
    }

    pub fn error(&self) -> &windows_core::Error {
        &self.error
    }
}

/// A Win32 invocation failure that retains resources needing cleanup retry.
#[derive(Debug)]
pub struct CallError {
    source: Error,
    cleanup_failures: Vec<CleanupFailure>,
}

impl CallError {
    pub fn message(&self) -> String {
        self.source.message()
    }

    pub fn source_error(&self) -> &Error {
        &self.source
    }

    /// Failures from the invocation. Resources remain in this list after a
    /// successful retry, with their shared owner marked closed.
    pub fn cleanup_failures(&self) -> &[CleanupFailure] {
        &self.cleanup_failures
    }

    /// Retries every failed cleanup, preserving failed owners for another retry.
    /// This never invokes the original native function again.
    pub fn retry_cleanup(&self) -> windows_core::Result<()> {
        let mut first_error = None;
        for failure in &self.cleanup_failures {
            if let Err(error) = failure.resource.close() {
                first_error.get_or_insert(error);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

impl From<Error> for CallError {
    fn from(source: Error) -> Self {
        Self {
            source,
            cleanup_failures: Vec::new(),
        }
    }
}

impl std::fmt::Display for CallError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message())
    }
}

impl std::error::Error for CallError {}

struct OwnedResult {
    target: ResultTarget,
    resource: Arc<OwnedResource>,
    cleanup_error: Option<windows_core::Error>,
}

#[derive(Default)]
pub(super) struct ResultOwners {
    owned: Vec<OwnedResult>,
    failures: Vec<CleanupFailure>,
}

impl ResultOwners {
    pub(super) fn reserve(&mut self, capacity: usize) -> Result<()> {
        self.owned
            .try_reserve_exact(capacity)
            .map_err(|_| super::out_of_memory("native result owners"))?;
        self.failures
            .try_reserve_exact(capacity)
            .map_err(|_| super::out_of_memory("native result cleanup failures"))
    }

    /// The caller has resolved the native result as defined and exclusively
    /// owned, with the exact cleanup matching its resource kind.
    pub(super) unsafe fn capture(
        &mut self,
        target: ResultTarget,
        value: &AbiValue,
        cleanup: &mut Cleanup,
    ) {
        if !cleanup.owns_resource() {
            return;
        }
        let AbiValue::Pointer(pointer) = value else {
            unreachable!("validated owned result is pointer-shaped");
        };
        if pointer.is_null() {
            return;
        }
        let resource = Arc::new(OwnedResource::new(*pointer as usize, *cleanup));
        self.owned.push(OwnedResult {
            target,
            resource,
            cleanup_error: None,
        });
        *cleanup = Cleanup::None;
    }

    pub(super) fn decode(
        &mut self,
        target: ResultTarget,
        policy: ResultPolicy,
    ) -> Option<Result<Value>> {
        let owned = self.owned.iter_mut().find(|owned| owned.target == target)?;
        let ResultPolicy::Defined {
            ownership: super::ResultOwnership::Owned { .. },
            delivery,
        } = policy
        else {
            unreachable!("captured owner has a defined owned result policy");
        };
        Some(match delivery {
            Delivery::Deliver => Ok(Value::Resource(Arc::clone(&owned.resource))),
            Delivery::Discard => match owned.resource.close() {
                Ok(()) => Ok(Value::Discarded),
                Err(error) => {
                    owned.cleanup_error = Some(error.clone());
                    Err(Error::WindowsError(error))
                }
            },
        })
    }

    pub(super) fn into_error(mut self, source: Error) -> CallError {
        for owned in self.owned {
            let error = owned.cleanup_error.or_else(|| owned.resource.close().err());
            if let Some(error) = error {
                self.failures.push(CleanupFailure {
                    target: owned.target,
                    resource: owned.resource,
                    error,
                });
            }
        }
        CallError {
            source,
            cleanup_failures: self.failures,
        }
    }
}

#[cfg(feature = "test-hooks")]
#[doc(hidden)]
pub fn test_cleanup_failure_plan() -> Result<Arc<super::CallPlan>> {
    use super::{
        CallContract, CallPlan, CallPlanSpec, CallingConvention, ResultContract, ResultOwnership,
        SuccessRule, Type,
    };
    use windows::Win32::{
        Foundation::{HANDLE_FLAG_PROTECT_FROM_CLOSE, SetHandleInformation},
        System::Threading::CreateEventW,
    };

    unsafe extern "system" fn protected_output() -> *mut std::ffi::c_void {
        let handle = unsafe { CreateEventW(None, true, false, None) }.unwrap();
        unsafe {
            SetHandleInformation(
                handle,
                HANDLE_FLAG_PROTECT_FROM_CLOSE.0,
                HANDLE_FLAG_PROTECT_FROM_CLOSE,
            )
        }
        .unwrap();
        handle.0
    }

    let spec = CallPlanSpec {
        dll: "kernel32.dll".into(),
        entry_point: "GetLastError".into(),
        parameters: Vec::new(),
        return_type: Some(Type::Handle),
        return_cleanup: Cleanup::CloseHandle,
        success_rule: SuccessRule::ReturnNonNull,
        capture_last_error: false,
        calling_convention: CallingConvention::System,
        parameter_aggregates: Vec::new(),
        return_aggregate: None,
    };
    let contract = CallContract::current(
        vec![ResultContract {
            target: ResultTarget::Return {},
            on_success: ResultPolicy::discarded(ResultOwnership::Owned {
                cleanup: dynwinrt_win32_contracts::Cleanup::CloseHandle,
            }),
            on_failure: ResultPolicy::Undefined {},
            overrides: Vec::new(),
        }],
        Vec::new(),
    );
    let mut plan = Arc::try_unwrap(unsafe { CallPlan::new_with_contract(spec, contract) }?)
        .expect("unshared test plan");
    plan.function = protected_output as *const () as usize;
    plan.dll = "test".into();
    plan.entry_point = "cleanupFailure".into();
    Ok(Arc::new(plan))
}
