// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use windows_core::GUID;

use super::MetadataTable;
use super::type_kind::*;

// ===========================================================================
// Signature / IID computation on MetadataTable
// ===========================================================================

impl MetadataTable {
    pub(crate) fn compute_parameterized_iid(&self, piid: &GUID, type_args: &[TypeKind]) -> GUID {
        let arg_sigs: Vec<String> = type_args
            .iter()
            .map(|a| self.signature_string_kind(*a))
            .collect();
        let sig = pinterface_signature_from_strings(&format_guid_braced(piid), &arg_sigs);
        let buf = windows_core::imp::ConstBuffer::from_slice(sig.as_bytes());
        GUID::from_signature(buf)
    }

    fn async_type_args(&self, kind: TypeKind) -> Vec<TypeKind> {
        match kind {
            TypeKind::IAsyncActionWithProgress(idx) | TypeKind::IAsyncOperation(idx) => {
                vec![self.get_inner_type(idx)]
            }
            TypeKind::IAsyncOperationWithProgress(idx) => {
                let (t, p) = self.get_inner_type_pair(idx);
                vec![t, p]
            }
            _ => vec![],
        }
    }

    pub(crate) fn signature_string_kind(&self, kind: TypeKind) -> String {
        self.try_signature_string_kind(kind)
            .expect("Type has no valid WinRT signature")
    }

    pub(crate) fn try_signature_string_kind(
        &self,
        kind: TypeKind,
    ) -> crate::result::Result<String> {
        self.try_signature_string_kind_impl(kind, true)
    }

    pub(crate) fn try_closed_signature_string_kind(
        &self,
        kind: TypeKind,
    ) -> crate::result::Result<String> {
        self.try_signature_string_kind_impl(kind, false)
    }

    fn try_signature_string_kind_impl(
        &self,
        kind: TypeKind,
        allow_open_generic: bool,
    ) -> crate::result::Result<String> {
        if let Some(sig) = kind.signature() {
            return Ok(sig.into());
        }
        match kind {
            TypeKind::Interface(iid) => Ok(format_guid_braced(&iid)),
            TypeKind::Generic { piid, .. } if allow_open_generic => Ok(format_guid_braced(&piid)),
            TypeKind::Delegate(iid) => Ok(format!("delegate({})", format_guid_braced(&iid))),
            TypeKind::RuntimeClass(idx) => {
                let (name, default_interface) = self.get_runtime_class(idx);
                Ok(format!(
                    "rc({};{})",
                    name,
                    self.try_signature_string_kind_impl(default_interface, false)?
                ))
            }
            TypeKind::Parameterized(idx) => {
                let (generic_def, args) = self.get_parameterized(idx);
                let piid = match generic_def {
                    TypeKind::Generic { piid, arity } if arity as usize == args.len() => piid,
                    TypeKind::Interface(iid) => iid,
                    _ => return Err(Self::invalid_signature(kind)),
                };
                let arg_sigs: crate::result::Result<Vec<String>> = args
                    .iter()
                    .map(|a| self.try_signature_string_kind_impl(*a, false))
                    .collect();
                Ok(pinterface_signature_from_strings(
                    &format_guid_braced(&piid),
                    &arg_sigs?,
                ))
            }
            TypeKind::IAsyncAction => Ok(format_guid_braced(&IASYNC_ACTION)),
            TypeKind::IAsyncActionWithProgress(_) => self.try_pinterface_signature(
                &IASYNC_ACTION_WITH_PROGRESS,
                &self.async_type_args(kind),
            ),
            TypeKind::IAsyncOperation(_) => {
                self.try_pinterface_signature(&IASYNC_OPERATION, &self.async_type_args(kind))
            }
            TypeKind::IAsyncOperationWithProgress(_) => self.try_pinterface_signature(
                &IASYNC_OPERATION_WITH_PROGRESS,
                &self.async_type_args(kind),
            ),
            TypeKind::Object => Ok("cinterface(IInspectable)".to_string()),
            TypeKind::HResult => Ok("i4".to_string()),
            TypeKind::Enum(idx) => {
                let name = self.get_enum_name(idx);
                Ok(format!("enum({};i4)", name))
            }
            TypeKind::Struct(idx) => {
                let entry = &self.structs.read().unwrap()[idx as usize];
                let name = &entry.name;
                let field_sigs: crate::result::Result<Vec<String>> = entry
                    .field_kinds
                    .iter()
                    .map(|k| self.try_signature_string_kind_impl(*k, false))
                    .collect();
                Ok(format!("struct({};{})", name, field_sigs?.join(";")))
            }
            _ => Err(Self::invalid_signature(kind)),
        }
    }

    fn try_pinterface_signature(
        &self,
        piid: &GUID,
        type_args: &[TypeKind],
    ) -> crate::result::Result<String> {
        let arg_sigs: crate::result::Result<Vec<String>> = type_args
            .iter()
            .map(|a| self.try_signature_string_kind_impl(*a, false))
            .collect();
        Ok(pinterface_signature_from_strings(
            &format_guid_braced(piid),
            &arg_sigs?,
        ))
    }

    pub(crate) fn iid_kind(&self, kind: TypeKind) -> Option<GUID> {
        match kind {
            TypeKind::Interface(iid) | TypeKind::Delegate(iid) => Some(iid),
            TypeKind::RuntimeClass(idx) => {
                let (_, default_interface) = self.get_runtime_class(idx);
                self.iid_kind(default_interface)
            }

            TypeKind::IAsyncAction => Some(IASYNC_ACTION),
            TypeKind::Parameterized(_)
            | TypeKind::IAsyncActionWithProgress(_)
            | TypeKind::IAsyncOperation(_)
            | TypeKind::IAsyncOperationWithProgress(_) => Some(self.compute_parameterized_iid(
                &self.parameterized_piid(kind),
                &self.parameterized_type_args(kind),
            )),
            _ => None,
        }
    }

    fn invalid_signature(kind: TypeKind) -> crate::result::Error {
        crate::result::Error::WindowsError(windows_core::Error::new(
            windows_core::HRESULT(0x80070057u32 as i32),
            &format!("Type {kind:?} has no valid WinRT signature"),
        ))
    }

    pub(crate) fn completed_handler_iid_kind(&self, kind: TypeKind) -> Option<GUID> {
        let handler_piid = match kind {
            TypeKind::IAsyncAction => return Some(ASYNC_ACTION_COMPLETED_HANDLER),
            TypeKind::IAsyncOperation(_) => ASYNC_OPERATION_COMPLETED_HANDLER,
            TypeKind::IAsyncActionWithProgress(_) => ASYNC_ACTION_WITH_PROGRESS_COMPLETED_HANDLER,
            TypeKind::IAsyncOperationWithProgress(_) => {
                ASYNC_OPERATION_WITH_PROGRESS_COMPLETED_HANDLER
            }
            _ => return None,
        };
        Some(self.compute_parameterized_iid(&handler_piid, &self.async_type_args(kind)))
    }

    pub(crate) fn progress_handler_iid_kind(&self, kind: TypeKind) -> Option<GUID> {
        let handler_piid = match kind {
            TypeKind::IAsyncActionWithProgress(_) => ASYNC_ACTION_PROGRESS_HANDLER,
            TypeKind::IAsyncOperationWithProgress(_) => ASYNC_OPERATION_PROGRESS_HANDLER,
            _ => return None,
        };
        // Progress handler type args:
        // - IAsyncActionWithProgress<P>: handler is AsyncActionProgressHandler<P> → [P]
        // - IAsyncOperationWithProgress<T,P>: handler is AsyncOperationProgressHandler<T,P> → [T, P]
        let progress_args = match kind {
            TypeKind::IAsyncActionWithProgress(idx) => vec![self.get_inner_type(idx)],
            TypeKind::IAsyncOperationWithProgress(idx) => {
                let (result_type, progress_type) = self.get_inner_type_pair(idx);
                vec![result_type, progress_type]
            }
            _ => return None,
        };
        Some(self.compute_parameterized_iid(&handler_piid, &progress_args))
    }

    fn parameterized_piid(&self, kind: TypeKind) -> GUID {
        match kind {
            TypeKind::Parameterized(idx) => {
                let (generic_def, _) = self.get_parameterized(idx);
                match generic_def {
                    TypeKind::Generic { piid, .. } | TypeKind::Interface(piid) => piid,
                    _ => panic!("Parameterized base must be Generic or Interface"),
                }
            }
            TypeKind::IAsyncActionWithProgress(_) => IASYNC_ACTION_WITH_PROGRESS,
            TypeKind::IAsyncOperation(_) => IASYNC_OPERATION,
            TypeKind::IAsyncOperationWithProgress(_) => IASYNC_OPERATION_WITH_PROGRESS,
            _ => panic!("Not a parameterized type: {:?}", kind),
        }
    }

    fn parameterized_type_args(&self, kind: TypeKind) -> Vec<TypeKind> {
        match kind {
            TypeKind::Parameterized(idx) => {
                let (_, args) = self.get_parameterized(idx);
                args
            }
            _ => self.async_type_args(kind),
        }
    }
}
