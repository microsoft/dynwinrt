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

    fn pinterface_signature(&self, piid: &GUID, type_args: &[TypeKind]) -> String {
        let arg_sigs: Vec<String> = type_args
            .iter()
            .map(|a| self.signature_string_kind(*a))
            .collect();
        pinterface_signature_from_strings(&format_guid_braced(piid), &arg_sigs)
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
        if let Some(sig) = kind.signature() {
            return sig.into();
        }
        match kind {
            TypeKind::Interface(iid) | TypeKind::Generic { piid: iid, .. } => {
                format_guid_braced(&iid)
            }
            TypeKind::Delegate(iid) => {
                format!("delegate({})", format_guid_braced(&iid))
            }
            TypeKind::RuntimeClass(idx) => {
                let (name, iid) = self.get_runtime_class(idx);
                format!("rc({};{})", name, format_guid_braced(&iid))
            }
            TypeKind::Parameterized(idx) => {
                let (generic_def, args) = self.get_parameterized(idx);
                let piid_sig = self.signature_string_kind(generic_def);
                let arg_sigs: Vec<String> = args
                    .iter()
                    .map(|a| self.signature_string_kind(*a))
                    .collect();
                pinterface_signature_from_strings(&piid_sig, &arg_sigs)
            }
            TypeKind::IAsyncAction => format_guid_braced(&IASYNC_ACTION),
            TypeKind::IAsyncActionWithProgress(_) => {
                self.pinterface_signature(&IASYNC_ACTION_WITH_PROGRESS, &self.async_type_args(kind))
            }
            TypeKind::IAsyncOperation(_) => {
                self.pinterface_signature(&IASYNC_OPERATION, &self.async_type_args(kind))
            }
            TypeKind::IAsyncOperationWithProgress(_) => self
                .pinterface_signature(&IASYNC_OPERATION_WITH_PROGRESS, &self.async_type_args(kind)),
            TypeKind::Object => "cinterface(IInspectable)".to_string(),
            TypeKind::HResult => "i4".to_string(),
            TypeKind::Enum(idx) => {
                let name = self.get_enum_name(idx);
                format!("enum({};i4)", name)
            }
            TypeKind::Struct(idx) => {
                let entry = &self.structs.read().unwrap()[idx as usize];
                let name = &entry.name;
                let field_sigs: Vec<String> = entry
                    .field_kinds
                    .iter()
                    .map(|k| self.signature_string_kind(*k))
                    .collect();
                format!("struct({};{})", name, field_sigs.join(";"))
            }
            _ => panic!("Type {:?} has no WinRT type signature", kind),
        }
    }

    pub(crate) fn iid_kind(&self, kind: TypeKind) -> Option<GUID> {
        match kind {
            TypeKind::Interface(iid) | TypeKind::Delegate(iid) => Some(iid),
            TypeKind::RuntimeClass(idx) => {
                let (_, iid) = self.get_runtime_class(idx);
                Some(iid)
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
