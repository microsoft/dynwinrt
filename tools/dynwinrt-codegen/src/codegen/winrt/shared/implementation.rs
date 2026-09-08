// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Authoritative, language-neutral reverse WinRT call contracts.
//!
//! Requirements are independent QI views, never inherited vtable prefixes.
//! Parameter order is native metadata order; output order is explicit out
//! parameters followed by the logical return.

use std::collections::HashSet;

use crate::meta::{InterfaceMeta, ParamDirection};
use crate::types::TypeMeta;

const IUNKNOWN: &str = "00000000-0000-0000-c000-000000000046";
const IINSPECTABLE: &str = "af86e2e0-b12d-4c6a-9c5a-d7aa65101e90";

#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ImplementationAbi {
    Scalar,
    HResult,
    Enum,
    Guid,
    HString,
    Reference,
    Struct(Vec<ImplementationType>),
    Array(Box<ImplementationType>),
}

/// The metadata is retained for language projection, but only after its full
/// native shape has passed validation. Renderers cannot see this type.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct ImplementationType {
    pub metadata: TypeMeta,
    pub abi: ImplementationAbi,
}

#[derive(Clone, Debug)]
pub(crate) struct ImplementationParameter {
    pub name: String,
    pub typ: ImplementationType,
    pub direction: ParamDirection,
    pub input_index: Option<usize>,
    pub output_index: Option<usize>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImplementationMethod {
    pub name: String,
    pub vtable_index: usize,
    pub parameters: Vec<ImplementationParameter>,
    pub return_type: Option<ImplementationType>,
    pub input_count: usize,
    pub output_count: usize,
    pub output_names: Vec<String>,
    pub handler_kind: ImplementationHandlerKind,
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum ImplementationHandlerKind {
    Method,
    Getter,
    Setter,
    Add,
    Remove,
}

#[derive(Clone, Debug)]
pub(crate) struct WinRtImplementationPlan {
    pub name: String,
    pub iid: String,
    pub required_interfaces: Vec<ImplementationType>,
    pub methods: Vec<ImplementationMethod>,
    pub delegates: Vec<ImplementationDelegate>,
}

#[derive(Clone, Debug)]
pub(crate) struct ImplementationDelegate {
    pub typ: ImplementationType,
    pub invoke: ImplementationMethod,
}

fn valid_iid(iid: &str) -> bool {
    iid.len() == 36
        && iid.chars().enumerate().all(|(index, ch)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                ch == '-'
            } else {
                ch.is_ascii_hexdigit()
            }
        })
        && iid != "00000000-0000-0000-0000-000000000000"
}

pub(crate) fn validate_type(typ: &TypeMeta, in_struct: bool) -> Result<ImplementationType, String> {
    let abi = match typ {
        TypeMeta::Bool
        | TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::Char16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64
        | TypeMeta::F32
        | TypeMeta::F64 => ImplementationAbi::Scalar,
        TypeMeta::Guid => ImplementationAbi::Guid,
        TypeMeta::String => ImplementationAbi::HString,
        TypeMeta::Object | TypeMeta::AsyncAction => ImplementationAbi::Reference,
        TypeMeta::Interface {
            namespace,
            name,
            iid,
        }
        | TypeMeta::Delegate {
            namespace,
            name,
            iid,
        } => {
            if !valid_iid(iid) {
                return Err(format!(
                    "{namespace}.{name} has an unresolved or invalid IID"
                ));
            }
            if name.contains('`') {
                return Err(format!("{namespace}.{name} is an open generic reference"));
            }
            ImplementationAbi::Reference
        }
        TypeMeta::RuntimeClass {
            namespace,
            name,
            default_interface,
        } => {
            let interface = default_interface
                .as_deref()
                .ok_or_else(|| format!("{namespace}.{name} has no resolved default interface"))?;
            let resolved = validate_type(interface, false)?;
            if !matches!(resolved.abi, ImplementationAbi::Reference)
                || matches!(interface, TypeMeta::Object | TypeMeta::RuntimeClass { .. })
            {
                return Err(format!(
                    "{namespace}.{name} has an invalid default interface"
                ));
            }
            ImplementationAbi::Reference
        }
        TypeMeta::Parameterized {
            namespace,
            name,
            piid,
            args,
        } => {
            if !valid_iid(piid) || args.is_empty() {
                return Err(format!(
                    "{namespace}.{name} has no resolved closed generic IID"
                ));
            }
            if namespace == "Windows.Foundation"
                && name.split('`').next() == Some("IReference")
                && !piid.eq_ignore_ascii_case("61c17706-2d65-11e0-9ae8-d48564015472")
            {
                return Err("IReference metadata has an incompatible PIID".into());
            }
            if let Some((_, arity)) = name.rsplit_once('`') {
                if arity.parse::<usize>().ok() != Some(args.len()) {
                    return Err(format!(
                        "{namespace}.{name} has incomplete generic arguments"
                    ));
                }
            }
            for arg in args {
                if matches!(arg, TypeMeta::Array(_)) {
                    return Err(format!(
                        "{namespace}.{name} has an unsupported array type argument"
                    ));
                }
                validate_type(arg, false)?;
            }
            if piid.eq_ignore_ascii_case("61c17706-2d65-11e0-9ae8-d48564015472")
                && (args.len() != 1
                    || matches!(
                        validate_type(&args[0], false)?.abi,
                        ImplementationAbi::Reference | ImplementationAbi::Array(_)
                    ))
            {
                return Err("IReference requires one supported value type".into());
            }
            ImplementationAbi::Reference
        }
        TypeMeta::AsyncOperation(result) | TypeMeta::AsyncActionWithProgress(result) => {
            validate_type(result, false)?;
            if matches!(result.as_ref(), TypeMeta::Array(_)) {
                return Err("async interfaces cannot have array type arguments".into());
            }
            ImplementationAbi::Reference
        }
        TypeMeta::AsyncOperationWithProgress(result, progress) => {
            for arg in [result, progress] {
                validate_type(arg, false)?;
                if matches!(arg.as_ref(), TypeMeta::Array(_)) {
                    return Err("async interfaces cannot have array type arguments".into());
                }
            }
            ImplementationAbi::Reference
        }
        TypeMeta::Enum {
            namespace,
            name,
            underlying,
            ..
        } => {
            if !matches!(underlying.as_ref(), TypeMeta::I32 | TypeMeta::U32) {
                return Err(format!(
                    "{namespace}.{name} does not have a 32-bit WinRT enum ABI"
                ));
            }
            ImplementationAbi::Enum
        }
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } if namespace == "Windows.Foundation" && name == "HResult" => {
            if fields.len() != 1 || fields[0].typ != TypeMeta::I32 {
                return Err("Windows.Foundation.HResult has an incomplete scalar layout".into());
            }
            ImplementationAbi::HResult
        }
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => {
            if fields.is_empty() {
                return Err(format!("{namespace}.{name} has no complete struct layout"));
            }
            let mut names = HashSet::new();
            let mut validated = Vec::new();
            for field in fields {
                if field.name.is_empty() || !names.insert(&field.name) {
                    return Err(format!("{namespace}.{name} has ambiguous struct fields"));
                }
                validated
                    .push(validate_type(&field.typ, true).map_err(|reason| {
                        format!("{namespace}.{name}.{}: {reason}", field.name)
                    })?);
            }
            ImplementationAbi::Struct(validated)
        }
        TypeMeta::Array(element) => {
            if in_struct || matches!(element.as_ref(), TypeMeta::Array(_)) {
                return Err(
                    "nested arrays and array struct fields have no supported WinRT layout".into(),
                );
            }
            ImplementationAbi::Array(Box::new(validate_type(element, false)?))
        }
    };
    Ok(ImplementationType {
        metadata: typ.clone(),
        abi,
    })
}

pub(crate) fn project_implementation(
    iface: &InterfaceMeta,
) -> Result<WinRtImplementationPlan, String> {
    let full_name = format!("{}.{}", iface.namespace, iface.name);
    let fail = |reason: String| format!("{full_name} cannot be implemented: {reason}");
    if iface.is_delegate() {
        return Err(fail(
            "delegates use their separate IUnknown-rooted callback API".into(),
        ));
    }
    if iface.generic_piid.is_some()
        || iface.generic_name.is_some()
        || !iface.generic_args.is_empty()
        || iface.implementation_metadata.is_generic_definition
        || iface.name.contains('`')
    {
        return Err(fail(
            "generic interface implementations are not supported".into(),
        ));
    }
    if !valid_iid(&iface.iid) {
        return Err(fail("interface IID is unresolved or invalid".into()));
    }
    if [IUNKNOWN, IINSPECTABLE]
        .iter()
        .any(|iid| iface.iid.eq_ignore_ascii_case(iid))
    {
        return Err(fail(
            "IUnknown and IInspectable are reserved host interfaces".into(),
        ));
    }
    if !iface.implementation_metadata.diagnostics.is_empty() {
        return Err(fail(iface.implementation_metadata.diagnostics.join("; ")));
    }
    let mut methods = Vec::new();
    for (position, method) in iface.methods.iter().enumerate() {
        if method.vtable_index != 6 + position {
            return Err(fail(format!(
                "{} must occupy WinRT slot {}, not {}; incomplete or non-WinRT vtable",
                method.name,
                6 + position,
                method.vtable_index
            )));
        }
        if method.name.is_empty() || method.name.starts_with('.') {
            return Err(fail(format!("unsupported method {}", method.name)));
        }
        let mut parameters = Vec::new();
        let mut input_count = 0;
        let mut output_count = 0;
        for parameter in &method.params {
            let typ = validate_type(&parameter.typ, false).map_err(|reason| {
                fail(format!(
                    "{} parameter {}: {reason}",
                    method.name, parameter.name
                ))
            })?;
            if parameter.direction == ParamDirection::OutFill
                && !matches!(typ.abi, ImplementationAbi::Array(_))
            {
                return Err(fail(format!(
                    "{} parameter {}: FillArray requires an array",
                    method.name, parameter.name
                )));
            }
            let input_index = if parameter.direction != ParamDirection::Out {
                let index = input_count;
                input_count += 1;
                Some(index)
            } else {
                None
            };
            let output_index = if parameter.direction != ParamDirection::In {
                let index = output_count;
                output_count += 1;
                Some(index)
            } else {
                None
            };
            parameters.push(ImplementationParameter {
                name: parameter.name.clone(),
                typ,
                direction: parameter.direction.clone(),
                input_index,
                output_index,
            });
        }
        let return_type = method
            .return_type
            .as_ref()
            .map(|typ| {
                validate_type(typ, false)
                    .map_err(|reason| fail(format!("{} return: {reason}", method.name)))
            })
            .transpose()?;
        output_count += usize::from(return_type.is_some());
        let mut output_names = parameters
            .iter()
            .filter_map(|parameter| {
                parameter.output_index.map(|index| {
                    if parameter.name.is_empty() {
                        format!("out{index}")
                    } else {
                        parameter.name.clone()
                    }
                })
            })
            .collect::<Vec<_>>();
        if return_type.is_some() {
            let occupied = output_names
                .iter()
                .map(|name| name.to_lowercase())
                .collect::<HashSet<_>>();
            let mut result_name = "result".to_string();
            while occupied.contains(&result_name) {
                result_name.push('_');
            }
            output_names.push(result_name);
        }
        let kind_count = [
            method.is_property_getter,
            method.is_property_setter,
            method.is_event_add,
            method.is_event_remove,
        ]
        .into_iter()
        .filter(|flag| *flag)
        .count();
        if kind_count > 1 {
            return Err(fail(format!(
                "{} has conflicting method semantics",
                method.name
            )));
        }
        methods.push(ImplementationMethod {
            name: method.name.clone(),
            vtable_index: method.vtable_index,
            parameters,
            return_type,
            input_count,
            output_count,
            output_names,
            handler_kind: if method.is_property_getter {
                ImplementationHandlerKind::Getter
            } else if method.is_property_setter {
                ImplementationHandlerKind::Setter
            } else if method.is_event_add {
                ImplementationHandlerKind::Add
            } else if method.is_event_remove {
                ImplementationHandlerKind::Remove
            } else {
                ImplementationHandlerKind::Method
            },
        });
    }
    let mut required_interfaces = Vec::new();
    let mut seen = HashSet::new();
    for typ in iface
        .base_interfaces
        .iter()
        .chain(&iface.implementation_metadata.required_interfaces)
    {
        let required = validate_type(typ, false)
            .map_err(|reason| fail(format!("required interface: {reason}")))?;
        if let TypeMeta::Parameterized {
            namespace, name, ..
        } = typ
        {
            return Err(fail(format!(
                "required generic interface {namespace}.{name} cannot be implemented by standalone factories"
            )));
        }
        if !matches!(
            typ,
            TypeMeta::Interface { .. } | TypeMeta::Parameterized { .. }
        ) {
            return Err(fail(
                "required interface has no independent interface view".into(),
            ));
        }
        if let TypeMeta::Interface { iid, .. } = typ {
            if [IUNKNOWN, IINSPECTABLE]
                .iter()
                .any(|reserved| iid.eq_ignore_ascii_case(reserved))
            {
                continue;
            }
            if iid.eq_ignore_ascii_case(&iface.iid) {
                return Err(fail("cyclic required interface closure".into()));
            }
        }
        if seen.insert(typ.type_identity()) {
            required_interfaces.push(required);
        }
    }
    let mut delegates = Vec::new();
    for delegate in &iface.implementation_metadata.delegates {
        let typ = validate_type(&delegate.typ, false).map_err(&fail)?;
        // Invoke uses slot 3, but the value and output contracts are identical
        // to WinRT methods. It is not a published interface implementation.
        let mut method = delegate.invoke.clone();
        method.vtable_index = 6;
        let delegate_iface = InterfaceMeta {
            namespace: iface.namespace.clone(),
            name: "DelegateInvokeContract".into(),
            iid: iface.iid.clone(),
            methods: vec![method],
            ..Default::default()
        };
        let mut invoke = project_implementation(&delegate_iface)?.methods.remove(0);
        invoke.vtable_index = 3;
        delegates.push(ImplementationDelegate { typ, invoke });
    }
    Ok(WinRtImplementationPlan {
        name: full_name,
        iid: iface.iid.clone(),
        required_interfaces,
        methods,
        delegates,
    })
}

/// Final language projection. ABI validation and all conversions have already
/// happened before a renderer receives these strings.
#[derive(Clone, Debug, Default)]
pub(crate) struct ImplementationProjection {
    pub support_code: String,
    pub declarations: String,
    pub factory_body: String,
    pub factory_declarations: String,
    pub supported: bool,
    pub exports: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{MethodMeta, ParamMeta};
    use crate::types::FieldMeta;

    fn interface(typ: TypeMeta) -> InterfaceMeta {
        InterfaceMeta {
            namespace: "Tests".into(),
            name: "IContract".into(),
            iid: "119ade03-d016-4c34-8813-4c2c3678dada".into(),
            methods: vec![MethodMeta {
                name: "RoundTrip".into(),
                vtable_index: 6,
                params: vec![ParamMeta {
                    name: "value".into(),
                    typ: typ.clone(),
                    direction: ParamDirection::In,
                }],
                return_type: Some(typ),
                ..Default::default()
            }],
            ..Default::default()
        }
    }

    fn managed() -> TypeMeta {
        TypeMeta::Interface {
            namespace: "Tests".into(),
            name: "IManaged".into(),
            iid: "12ace702-8ff7-4e34-95c4-04e13f64c885".into(),
        }
    }

    #[test]
    fn implementation_matrix_validates_scalar_reference_struct_and_array_contracts() {
        let structure = TypeMeta::Struct {
            namespace: "Tests".into(),
            name: "Record".into(),
            fields: vec![
                FieldMeta {
                    name: "Text".into(),
                    typ: TypeMeta::String,
                },
                FieldMeta {
                    name: "Reference".into(),
                    typ: managed(),
                },
                FieldMeta {
                    name: "Value".into(),
                    typ: TypeMeta::Struct {
                        namespace: "Tests".into(),
                        name: "Inner".into(),
                        fields: vec![FieldMeta {
                            name: "Number".into(),
                            typ: TypeMeta::U64,
                        }],
                    },
                },
            ],
        };
        let enum_type = TypeMeta::Enum {
            namespace: "Tests".into(),
            name: "Choice".into(),
            underlying: Box::new(TypeMeta::I32),
            members: vec![],
            is_flags: false,
            doc: None,
            deprecated: None,
        };
        let types = vec![
            TypeMeta::Bool,
            TypeMeta::I8,
            TypeMeta::U8,
            TypeMeta::I16,
            TypeMeta::U16,
            TypeMeta::Char16,
            TypeMeta::I32,
            TypeMeta::U32,
            TypeMeta::I64,
            TypeMeta::U64,
            TypeMeta::F32,
            TypeMeta::F64,
            TypeMeta::String,
            TypeMeta::Guid,
            TypeMeta::Object,
            TypeMeta::AsyncAction,
            TypeMeta::AsyncOperation(Box::new(TypeMeta::String)),
            TypeMeta::AsyncActionWithProgress(Box::new(TypeMeta::U32)),
            TypeMeta::AsyncOperationWithProgress(Box::new(TypeMeta::Guid), Box::new(TypeMeta::U32)),
            enum_type,
            managed(),
            structure,
            TypeMeta::RuntimeClass {
                namespace: "Tests".into(),
                name: "Managed".into(),
                default_interface: Some(Box::new(managed())),
            },
            TypeMeta::Parameterized {
                namespace: "Windows.Foundation".into(),
                name: "IReference`1".into(),
                piid: "61c17706-2d65-11e0-9ae8-d48564015472".into(),
                args: vec![TypeMeta::I32],
            },
            TypeMeta::Struct {
                namespace: "Windows.Foundation".into(),
                name: "HResult".into(),
                fields: vec![FieldMeta {
                    name: "Value".into(),
                    typ: TypeMeta::I32,
                }],
            },
        ];
        for typ in types {
            let plan = project_implementation(&interface(typ.clone()))
                .unwrap_or_else(|e| panic!("{typ:?}: {e}"));
            assert_eq!(plan.methods[0].input_count, 1);
            assert_eq!(plan.methods[0].output_count, 1);
            project_implementation(&interface(TypeMeta::Array(Box::new(typ))))
                .expect("supported array element");
        }
    }

    #[test]
    fn implementation_keeps_native_parameter_order_and_explicit_output_order() {
        let mut iface = interface(TypeMeta::I32);
        iface.methods[0].params = vec![
            ParamMeta {
                name: "firstOut".into(),
                typ: TypeMeta::String,
                direction: ParamDirection::Out,
            },
            ParamMeta {
                name: "input".into(),
                typ: TypeMeta::Bool,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "buffer".into(),
                typ: TypeMeta::Array(Box::new(TypeMeta::Guid)),
                direction: ParamDirection::OutFill,
            },
            ParamMeta {
                name: "lastInput".into(),
                typ: TypeMeta::U64,
                direction: ParamDirection::In,
            },
            ParamMeta {
                name: "lastOut".into(),
                typ: managed(),
                direction: ParamDirection::Out,
            },
        ];
        let plan = project_implementation(&iface).unwrap();
        let method = &plan.methods[0];
        assert_eq!(method.vtable_index, 6);
        assert_eq!(method.input_count, 3);
        assert_eq!(method.output_count, 4);
        assert_eq!(
            method.output_names,
            ["firstOut", "buffer", "lastOut", "result"]
        );
        assert_eq!(
            method
                .parameters
                .iter()
                .map(|p| p.input_index)
                .collect::<Vec<_>>(),
            [None, Some(0), Some(1), Some(2), None]
        );
        assert_eq!(
            method
                .parameters
                .iter()
                .map(|p| p.output_index)
                .collect::<Vec<_>>(),
            [Some(0), None, Some(1), None, Some(2)]
        );
        assert_eq!(method.return_type.as_ref().unwrap().metadata, TypeMeta::I32);
    }

    #[test]
    fn implementation_requirements_are_independent_views_not_vtable_prefixes() {
        let mut iface = interface(TypeMeta::String);
        iface.base_interfaces.push(managed());
        let ancestor = TypeMeta::Interface {
            namespace: "Tests".into(),
            name: "IAncestor".into(),
            iid: "5c011b9b-e33b-40e7-b787-9442224b9e51".into(),
        };
        iface
            .implementation_metadata
            .required_interfaces
            .extend([managed(), ancestor]);
        let plan = project_implementation(&iface).unwrap();
        assert_eq!(plan.required_interfaces.len(), 2);
        assert_eq!(plan.methods.len(), 1);
        assert_eq!(plan.methods[0].vtable_index, 6);
    }

    #[test]
    fn implementation_rejects_unknown_layouts_iids_and_nested_arrays_as_whole_interfaces() {
        let bad = vec![
            (
                TypeMeta::Interface {
                    namespace: "Tests".into(),
                    name: "IMissing".into(),
                    iid: String::new(),
                },
                "IID",
            ),
            (
                TypeMeta::RuntimeClass {
                    namespace: "Tests".into(),
                    name: "Missing".into(),
                    default_interface: None,
                },
                "default interface",
            ),
            (
                TypeMeta::Struct {
                    namespace: "Tests".into(),
                    name: "Unknown".into(),
                    fields: vec![],
                },
                "struct layout",
            ),
            (
                TypeMeta::Array(Box::new(TypeMeta::Array(Box::new(TypeMeta::I32)))),
                "nested arrays",
            ),
            (
                TypeMeta::Parameterized {
                    namespace: "Tests".into(),
                    name: "IOpen`2".into(),
                    piid: "c6c7fcce-aafa-4937-b9e8-6d98dcae7f3d".into(),
                    args: vec![TypeMeta::I32],
                },
                "generic arguments",
            ),
        ];
        for (typ, expected) in bad {
            let mut iface = interface(TypeMeta::String);
            iface.methods.push(MethodMeta {
                name: "Bad".into(),
                vtable_index: 7,
                return_type: Some(typ),
                ..Default::default()
            });
            let error = project_implementation(&iface).unwrap_err();
            assert!(error.contains(expected) && error.contains("Bad"), "{error}");
        }
    }

    #[test]
    fn implementation_rejects_generic_roots_host_roots_partial_vtables_and_metadata_errors() {
        let mut iface = interface(TypeMeta::String);
        iface.generic_piid = Some(iface.iid.clone());
        assert!(
            project_implementation(&iface)
                .unwrap_err()
                .contains("generic interface")
        );
        iface.generic_piid = None;
        iface.methods[0].vtable_index = 3;
        assert!(
            project_implementation(&iface)
                .unwrap_err()
                .contains("WinRT slot 6")
        );
        iface.methods[0].vtable_index = 6;
        iface.iid = IINSPECTABLE.into();
        assert!(
            project_implementation(&iface)
                .unwrap_err()
                .contains("reserved")
        );
        iface = interface(TypeMeta::String);
        iface
            .implementation_metadata
            .diagnostics
            .push("unknown pointer contract".into());
        assert!(
            project_implementation(&iface)
                .unwrap_err()
                .contains("unknown pointer contract")
        );
        iface.implementation_metadata.diagnostics.clear();
        iface.methods[0].params[0].direction = ParamDirection::OutFill;
        assert!(
            project_implementation(&iface)
                .unwrap_err()
                .contains("FillArray requires an array")
        );
    }
}
