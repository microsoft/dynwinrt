// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python projection of the same validated reverse-call plan used by JavaScript.

use std::collections::HashSet;

use crate::codegen::winrt::shared::implementation::{
    ImplementationAbi, ImplementationHandlerKind, ImplementationMethod, ImplementationProjection,
    ImplementationType, WinRtImplementationPlan, project_implementation, validate_type,
};
use crate::codegen::winrt::shared::imports::ireference_inner_type;
use crate::meta::{InterfaceMeta, ParamDirection};
use crate::types::TypeMeta;

use super::naming::{PythonProjectionContext, to_snake_case};
use super::native_types::{FoundationType, foundation_type};
use super::signature::{py_convert_return, py_dynwinrt_type, py_runtime_type_symbol};

pub(super) const IMPORTS: &str = "\
from typing import Protocol, TypedDict
from dynwinrt import (
    DynWinRTInterfacePlan, DynWinRTImplementationMethod,
    DynWinRTImplementation, DynWinRTImplementationDescriptor,
    DynWinRTImplementationHandle,
)
";
pub(super) const HELPER_IMPORTS: &str = "\
from ._runtime import (
    _implementation_check, _implementation_field, _implementation_array,
    _implementation_reference, _implementation_sync, _implementation_handler,
)
";

pub(super) const HELPERS: &str = r#"
import inspect as _implementation_inspect


def _implementation_check(value, valid, label):
    if not valid:
        raise TypeError(f'{label}: invalid implementation result')
    return value


def _implementation_field(value, name, label):
    if not isinstance(value, dict) or name not in value:
        raise TypeError(f'{label}: expected a result dict containing {name}')
    return value[name]


def _implementation_array(value, label, bytes_allowed=False, capacity=None):
    if not isinstance(value, (list, tuple)) and not (bytes_allowed and isinstance(value, (bytes, bytearray))):
        raise TypeError(f'{label}: expected an array')
    if capacity is not None and len(value) != capacity:
        raise ValueError(f'{label}: FillArray result must match capacity {capacity}')
    return list(value)


def _implementation_reference(value, iid, label):
    if value is None:
        return DynWinRTValue.null_value()
    raw = getattr(value, '_obj', value)
    if not isinstance(raw, DynWinRTValue):
        raise TypeError(f'{label}: expected a managed WinRT value or None')
    return DynWinRTValue.null_value() if raw.is_null() else raw.cast(iid)


def _implementation_sync(value, label):
    if _implementation_inspect.isawaitable(value) or _implementation_inspect.isasyncgen(value):
        if _implementation_inspect.iscoroutine(value):
            value.close()
        raise TypeError(f'{label}: implementation callbacks must return synchronously, not a coroutine or awaitable')
    return value


def _implementation_handler(handlers, name, count, label):
    try:
        member = _implementation_inspect.getattr_static(handlers, name)
    except AttributeError:
        raise TypeError(f'{label}: missing synchronous handler {name}') from None
    if isinstance(member, property):
        raise TypeError(f'{label}: property accessors are not callback handlers: {name}')
    callback = getattr(handlers, name)
    if not callable(callback):
        raise TypeError(f'{label}: missing synchronous handler {name}')
    if (_implementation_inspect.iscoroutinefunction(callback)
            or _implementation_inspect.isasyncgenfunction(callback)
            or _implementation_inspect.iscoroutinefunction(getattr(callback, '__call__', None))
            or _implementation_inspect.isasyncgenfunction(getattr(callback, '__call__', None))):
        raise TypeError(f'{label}.{name}: async handlers are not supported')
    try:
        signature = _implementation_inspect.signature(callback)
    except ValueError:
        pass
    else:
        try:
            signature.bind(*([None] * count))
        except TypeError as error:
            raise TypeError(f'{label}.{name}: invalid handler signature: {error}') from None
    return callback

"#;

fn quote(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization")
}

fn handler_name(method: &ImplementationMethod) -> String {
    let (prefix, raw) = match method.handler_kind {
        ImplementationHandlerKind::Method => return to_snake_case(&method.name),
        ImplementationHandlerKind::Getter => ("get_", "get_"),
        ImplementationHandlerKind::Setter => ("set_", "put_"),
        ImplementationHandlerKind::Add => ("add_", "add_"),
        ImplementationHandlerKind::Remove => ("remove_", "remove_"),
    };
    format!(
        "{prefix}{}",
        to_snake_case(method.name.strip_prefix(raw).unwrap_or(&method.name))
    )
}

fn input_name(name: &str, index: usize, fill: bool) -> String {
    let name = if name.is_empty() {
        format!("arg{index}")
    } else {
        to_snake_case(name)
    };
    if fill {
        format!("{name}_capacity")
    } else {
        name
    }
}

fn output_names(method: &ImplementationMethod) -> Result<Vec<String>, String> {
    let names = method
        .output_names
        .iter()
        .map(|name| to_snake_case(name))
        .collect::<Vec<_>>();
    if names.iter().collect::<HashSet<_>>().len() != names.len() {
        return Err(format!("{} has colliding output names", method.name));
    }
    Ok(names)
}

fn collect_structs(typ: &ImplementationType, output: &mut Vec<ImplementationType>) {
    match &typ.abi {
        ImplementationAbi::Struct(fields) => {
            if !output
                .iter()
                .any(|candidate| candidate.metadata == typ.metadata)
            {
                output.push(typ.clone());
                for field in fields {
                    collect_structs(field, output);
                }
            }
        }
        ImplementationAbi::Array(element) => collect_structs(element, output),
        ImplementationAbi::Reference => {
            if let Some(inner) = ireference_inner_type(&typ.metadata) {
                collect_structs(
                    &validate_type(inner, false).expect("validated IReference"),
                    output,
                );
            }
        }
        _ => {}
    }
}

struct Projector<'a> {
    context: &'a PythonProjectionContext,
    plan: &'a WinRtImplementationPlan,
    prefix: String,
    structs: Vec<ImplementationType>,
}

impl Projector<'_> {
    fn delegate_index(&self, typ: &TypeMeta) -> Option<usize> {
        self.plan
            .delegates
            .iter()
            .position(|delegate| delegate.typ.metadata == *typ)
    }

    fn native(&self, typ: &ImplementationType) -> String {
        match &typ.abi {
            ImplementationAbi::Array(element) => {
                format!("DynWinRTType.array_type({})", self.native(element))
            }
            ImplementationAbi::Struct(fields) => {
                let TypeMeta::Struct {
                    namespace, name, ..
                } = &typ.metadata
                else {
                    unreachable!()
                };
                format!(
                    "DynWinRTType.struct_type({}, [{}])",
                    quote(&format!("{namespace}.{name}")),
                    fields
                        .iter()
                        .map(|field| self.native(field))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }
            ImplementationAbi::Reference => match &typ.metadata {
                TypeMeta::Interface { iid, .. } | TypeMeta::Delegate { iid, .. } => {
                    let kind = if self.delegate_index(&typ.metadata).is_some()
                        || matches!(typ.metadata, TypeMeta::Delegate { .. })
                    {
                        "delegate"
                    } else {
                        "interface"
                    };
                    format!("DynWinRTType.{kind}(WinGUID.parse({}))", quote(iid))
                }
                TypeMeta::RuntimeClass {
                    namespace,
                    name,
                    default_interface,
                } => {
                    let default = validate_type(
                        default_interface
                            .as_deref()
                            .expect("validated default interface"),
                        false,
                    )
                    .expect("validated type");
                    format!(
                        "DynWinRTType.runtime_class({}, {})",
                        quote(&format!("{namespace}.{name}")),
                        self.native(&default)
                    )
                }
                TypeMeta::Parameterized { piid, args, .. } => format!(
                    "DynWinRTType.parameterized(WinGUID.parse({}), [{}])",
                    quote(piid),
                    args.iter()
                        .map(|typ| self
                            .native(&validate_type(typ, false).expect("validated argument")))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                TypeMeta::Object
                | TypeMeta::AsyncAction
                | TypeMeta::AsyncActionWithProgress(_)
                | TypeMeta::AsyncOperation(_)
                | TypeMeta::AsyncOperationWithProgress(_, _) => py_dynwinrt_type(&typ.metadata),
                _ => unreachable!("validated reference"),
            },
            ImplementationAbi::Scalar
            | ImplementationAbi::HResult
            | ImplementationAbi::Enum
            | ImplementationAbi::Guid
            | ImplementationAbi::HString => py_dynwinrt_type(&typ.metadata),
        }
    }

    fn iid(&self, typ: &ImplementationType) -> String {
        if typ.metadata == TypeMeta::Object {
            "WinGUID.parse('af86e2e0-b12d-4c6a-9c5a-d7aa65101e90')".into()
        } else {
            format!("{}.iid()", self.native(typ))
        }
    }

    fn annotation(&self, typ: &ImplementationType, writing: bool) -> String {
        if let Some(index) = self.delegate_index(&typ.metadata) {
            return format!(
                "{}Delegate{index}{} | None",
                self.prefix,
                if writing { " | DynWinRTValue" } else { "" }
            );
        }
        if let Some(inner) = ireference_inner_type(&typ.metadata) {
            return format!(
                "{} | None",
                self.annotation(
                    &validate_type(inner, false).expect("validated IReference"),
                    writing
                )
            );
        }
        match &typ.abi {
            ImplementationAbi::Array(element) => {
                let array = format!("list[{}]", self.annotation(element, writing));
                if writing && element.metadata == TypeMeta::U8 {
                    format!("{array} | bytes | bytearray")
                } else {
                    array
                }
            }
            ImplementationAbi::Struct(_) => match foundation_type(&typ.metadata) {
                Some(FoundationType::DateTime) => "datetime".into(),
                Some(FoundationType::TimeSpan) => "timedelta".into(),
                None => self.context.reference_name_for_type(&typ.metadata),
            },
            ImplementationAbi::HResult => "int".into(),
            ImplementationAbi::Enum => {
                if self.context.is_known_type(&typ.metadata) {
                    self.context.reference_name_for_type(&typ.metadata)
                } else {
                    "int".into()
                }
            }
            ImplementationAbi::Reference => {
                let name = match &typ.metadata {
                    TypeMeta::RuntimeClass { .. }
                    | TypeMeta::Interface { .. }
                    | TypeMeta::Parameterized { .. }
                        if self.context.is_known_type(&typ.metadata) =>
                    {
                        self.context.reference_name_for_type(&typ.metadata)
                    }
                    _ => "DynWinRTValue".into(),
                };
                format!("{name} | None")
            }
            ImplementationAbi::Scalar | ImplementationAbi::Guid | ImplementationAbi::HString => {
                match typ.metadata {
                    TypeMeta::Bool => "bool",
                    TypeMeta::String | TypeMeta::Char16 => "str",
                    TypeMeta::Guid => "UUID",
                    TypeMeta::F32 | TypeMeta::F64 => "float",
                    _ => "int",
                }
                .into()
            }
        }
    }

    fn struct_symbol(&self, typ: &TypeMeta) -> String {
        let TypeMeta::Struct { name, .. } = typ else {
            unreachable!()
        };
        if self.context.is_packaged() {
            py_runtime_type_symbol(self.context, typ, name)
        } else {
            name.clone()
        }
    }

    fn read(&self, typ: &ImplementationType, value: &str) -> String {
        if let Some(index) = self.delegate_index(&typ.metadata) {
            return format!("_{}Delegate{index}({value})", self.prefix);
        }
        if let Some(inner) = ireference_inner_type(&typ.metadata) {
            let inner = validate_type(inner, false).expect("validated IReference value");
            let native = format!(
                "DynWinRTType.register_interface('__dynwinrt_reverse_reference_' + str(iid), iid).add_method('get_Value', DynWinRTMethodSig().add_out({})).method(6).invoke(v.cast(iid), [])",
                self.native(&inner)
            );
            return format!(
                "(lambda v: None if v.is_null() else (lambda iid: {})({}))({value})",
                self.read(&inner, &native),
                self.iid(typ)
            );
        }
        match &typ.abi {
            ImplementationAbi::Array(element) => format!(
                "[{} for v in {value}.as_array().to_values()]",
                self.read(element, "v")
            ),
            ImplementationAbi::Struct(fields) if foundation_type(&typ.metadata).is_none() => {
                let TypeMeta::Struct {
                    fields: metadata, ..
                } = &typ.metadata
                else {
                    unreachable!()
                };
                let fields = metadata
                    .iter()
                    .zip(fields)
                    .enumerate()
                    .map(|(index, (field, typ))| {
                        let expr = match &typ.abi {
                            ImplementationAbi::Reference => {
                                self.read(typ, &format!("s.get_object({index})"))
                            }
                            ImplementationAbi::Struct(_) => {
                                self.read(typ, &format!("s.get_struct({index}).to_value()"))
                            }
                            _ => super::structs::py_struct_field_getter(
                                self.context,
                                &typ.metadata,
                                index,
                            ),
                        };
                        format!("{}={expr}", to_snake_case(&field.name))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!(
                    "(lambda s: {}({fields}))({value}.as_struct())",
                    self.struct_symbol(&typ.metadata)
                )
            }
            ImplementationAbi::Reference if typ.metadata.is_async() => {
                format!("(lambda v: None if v.is_null() else v)({value})")
            }
            _ => py_convert_return(value, Some(&typ.metadata), false, self.context),
        }
    }

    fn check_scalar(&self, typ: &ImplementationType, value: &str, label: &str) -> String {
        let integer = |min: &str, max: &str| {
            format!("isinstance(v, int) and not isinstance(v, bool) and {min} <= v <= {max}")
        };
        let check = match &typ.metadata {
            TypeMeta::Bool => "isinstance(v, bool)".into(),
            TypeMeta::String => "isinstance(v, str)".into(),
            TypeMeta::Char16 => "isinstance(v, str) and len(v) == 1 and ord(v) <= 65535".into(),
            TypeMeta::Guid => "isinstance(v, UUID)".into(),
            TypeMeta::I8 => integer("-128", "127"), TypeMeta::U8 => integer("0", "255"),
            TypeMeta::I16 => integer("-32768", "32767"), TypeMeta::U16 => integer("0", "65535"),
            TypeMeta::I32 | TypeMeta::Enum { .. } | TypeMeta::Struct { .. } => integer("-2147483648", "2147483647"),
            TypeMeta::U32 => integer("0", "4294967295"),
            TypeMeta::I64 => integer("-9223372036854775808", "9223372036854775807"),
            TypeMeta::U64 => integer("0", "18446744073709551615"),
            TypeMeta::F32 => "isinstance(v, (int, float)) and not isinstance(v, bool) and (not __import__('math').isfinite(v) or abs(v) <= 3.4028234663852886e38)".into(),
            TypeMeta::F64 => "isinstance(v, (int, float)) and not isinstance(v, bool)".into(),
            _ => unreachable!("validated scalar"),
        };
        format!(
            "(lambda v: _implementation_check(v, {check}, {}))({value})",
            quote(label)
        )
    }

    fn write(
        &self,
        typ: &ImplementationType,
        value: &str,
        label: &str,
        capacity: Option<&str>,
    ) -> String {
        if let Some(inner) = ireference_inner_type(&typ.metadata) {
            let inner = validate_type(inner, false).expect("validated IReference");
            return format!(
                "(lambda v: DynWinRTValue.null_value() if v is None else _implementation_reference(v, {}, {}) if isinstance(getattr(v, '_obj', v), DynWinRTValue) else DynWinRTValue.box_reference({}, {}))({value})",
                self.iid(typ),
                quote(label),
                self.write(&inner, "v", label, None),
                self.native(&inner)
            );
        }
        match &typ.abi {
            ImplementationAbi::Reference => format!(
                "_implementation_reference({value}, {}, {})",
                self.iid(typ),
                quote(label)
            ),
            ImplementationAbi::Array(element)
                if matches!(element.abi, ImplementationAbi::HResult) =>
            {
                format!(
                    "DynWinRTArray.from_values([{} for v in _implementation_array({value}, {}, False, {})], {}).to_value()",
                    self.write(element, "v", &format!("{label}[]"), None),
                    quote(label),
                    capacity.unwrap_or("None"),
                    self.native(element)
                )
            }
            ImplementationAbi::Array(element) => format!(
                "DynWinRTArray.from_object_values([{} for v in _implementation_array({value}, {}, {}, {})], {}).to_value()",
                self.write(element, "v", &format!("{label}[]"), None),
                quote(label),
                if element.metadata == TypeMeta::U8 {
                    "True"
                } else {
                    "False"
                },
                capacity.unwrap_or("None"),
                self.native(element)
            ),
            ImplementationAbi::Struct(_) => {
                let index = self
                    .structs
                    .iter()
                    .position(|candidate| candidate == typ)
                    .expect("collected struct");
                format!("_{}Struct{index}({value}, {})", self.prefix, quote(label))
            }
            ImplementationAbi::HResult => format!(
                "DynWinRTValue.from_hresult({})",
                self.check_scalar(typ, value, label)
            ),
            ImplementationAbi::Scalar
            | ImplementationAbi::Enum
            | ImplementationAbi::Guid
            | ImplementationAbi::HString => {
                let value = self.check_scalar(typ, value, label);
                let constructor = match typ.metadata {
                    TypeMeta::Bool => "from_bool",
                    TypeMeta::I8 => "from_i8",
                    TypeMeta::U8 => "from_u8",
                    TypeMeta::I16 => "from_i16",
                    TypeMeta::U16 => "from_u16",
                    TypeMeta::Char16 => return format!("DynWinRTValue.from_u16(ord({value}))"),
                    TypeMeta::I32 => "from_i32",
                    TypeMeta::U32 => "from_u32",
                    TypeMeta::I64 => "from_i64",
                    TypeMeta::U64 => "from_u64",
                    TypeMeta::F32 => "from_f32",
                    TypeMeta::F64 => "from_f64",
                    TypeMeta::String => "from_hstring",
                    TypeMeta::Guid => {
                        return format!("DynWinRTValue.from_guid(_dynwinrt_guid({value}))");
                    }
                    TypeMeta::Enum { .. } => {
                        return format!(
                            "DynWinRTValue.enum_value({}, int({value}))",
                            self.native(typ)
                        );
                    }
                    _ => unreachable!("validated scalar"),
                };
                format!("DynWinRTValue.{constructor}({value})")
            }
        }
    }

    fn signature(&self, method: &ImplementationMethod) -> String {
        let mut signature = "DynWinRTMethodSig()".to_string();
        for parameter in &method.parameters {
            let add = match parameter.direction {
                ParamDirection::In => "add_in",
                ParamDirection::Out => "add_out",
                ParamDirection::OutFill => "add_out_fill",
            };
            signature.push_str(&format!(".{add}({})", self.native(&parameter.typ)));
        }
        if let Some(typ) = &method.return_type {
            signature.push_str(&format!(".add_out({})", self.native(typ)));
        }
        signature
    }

    fn default_value(&self, typ: &ImplementationType) -> String {
        match &typ.abi {
            ImplementationAbi::Reference => "DynWinRTValue.null_value()".into(),
            ImplementationAbi::Struct(_) => {
                format!("DynWinRTStruct.create({}).to_value()", self.native(typ))
            }
            ImplementationAbi::Array(_) => unreachable!("nested arrays were rejected"),
            ImplementationAbi::Enum => format!("DynWinRTValue.enum_value({}, 0)", self.native(typ)),
            _ => self.write(
                typ,
                match typ.metadata {
                    TypeMeta::Bool => "False",
                    TypeMeta::String => "''",
                    TypeMeta::Char16 => "'\\0'",
                    TypeMeta::Guid => "UUID(int=0)",
                    _ => "0",
                },
                "delegate fill storage",
                None,
            ),
        }
    }

    fn delegate_fill_input(&self, typ: &ImplementationType, capacity: &str) -> String {
        let ImplementationAbi::Array(element) = &typ.abi else {
            unreachable!("validated FillArray")
        };
        format!(
            "DynWinRTArray.{}([{} for _ in range(_implementation_check({capacity}, isinstance({capacity}, int) and not isinstance({capacity}, bool) and 0 <= {capacity} <= 4294967295, 'delegate capacity'))], {}).to_value()",
            if matches!(element.abi, ImplementationAbi::HResult) {
                "from_values"
            } else {
                "from_object_values"
            },
            self.default_value(element),
            self.native(element)
        )
    }

    fn parameters(&self, method: &ImplementationMethod, writing: bool) -> Result<String, String> {
        let mut names = HashSet::new();
        let mut params = Vec::new();
        for parameter in &method.parameters {
            if let Some(index) = parameter.input_index {
                let fill = parameter.direction == ParamDirection::OutFill;
                let mut name = input_name(&parameter.name, index, fill);
                if name == "self" {
                    name.push('_');
                }
                if !names.insert(name.clone()) {
                    return Err(format!(
                        "{} has colliding handler parameter names",
                        method.name
                    ));
                }
                params.push(format!(
                    "{name}: {}",
                    if fill {
                        "int".into()
                    } else {
                        self.annotation(&parameter.typ, writing)
                    }
                ));
            }
        }
        Ok(params.join(", "))
    }

    fn result_name(&self, method: &ImplementationMethod) -> String {
        format!("{}{}Result", self.prefix, method.name)
    }

    fn result_type(&self, method: &ImplementationMethod, writing: bool) -> String {
        let types = method
            .parameters
            .iter()
            .filter(|p| p.output_index.is_some())
            .map(|p| &p.typ)
            .chain(method.return_type.iter())
            .collect::<Vec<_>>();
        match types.as_slice() {
            [] => "None".into(),
            [typ] => self.annotation(typ, writing),
            _ => self.result_name(method),
        }
    }
}

pub(super) fn project(
    context: &PythonProjectionContext,
    iface: &InterfaceMeta,
) -> ImplementationProjection {
    project_implementation(iface).and_then(|plan| {
        for method in &iface.methods {
            let name = if method.is_property_getter {
                method.name.strip_prefix("get_").unwrap_or(&method.name)
            } else if method.is_property_setter {
                method.name.strip_prefix("put_").unwrap_or(&method.name)
            } else { &method.name };
            if ["implementation", "implement", "from_implementation"].contains(&to_snake_case(name).as_str()) {
                return Err(format!("{} cannot be implemented: factory name conflicts with outbound member {name}", plan.name));
            }
        }
        project_validated(context, iface, &plan)
    })
        .unwrap_or_else(|reason| ImplementationProjection {
            factory_body: format!(
                "    @staticmethod\n    def implementation(handlers):\n        raise TypeError({0})\n\n    @staticmethod\n    def implement(handlers, *additional):\n        raise TypeError({0})\n\n", quote(&reason)
            ),
            factory_declarations: format!("    # {}\n", reason.replace('\n', " ")),
            ..Default::default()
        })
}

fn project_validated(
    context: &PythonProjectionContext,
    iface: &InterfaceMeta,
    plan: &WinRtImplementationPlan,
) -> Result<ImplementationProjection, String> {
    let mut structs = Vec::new();
    for method in plan
        .methods
        .iter()
        .chain(plan.delegates.iter().map(|delegate| &delegate.invoke))
    {
        for typ in method
            .parameters
            .iter()
            .map(|parameter| &parameter.typ)
            .chain(method.return_type.iter())
        {
            collect_structs(typ, &mut structs);
        }
    }
    let projector = Projector {
        context,
        plan,
        structs,
        prefix: format!("{}Implementation", iface.name),
    };
    let mut support = String::from(HELPER_IMPORTS);
    let mut declarations = String::new();
    let mut result_names = HashSet::new();
    for (result_name, method) in plan
        .methods
        .iter()
        .map(|method| (projector.result_name(method), method))
        .chain(plan.delegates.iter().enumerate().map(|(index, delegate)| {
            (
                format!("{}Delegate{index}Result", projector.prefix),
                &delegate.invoke,
            )
        }))
    {
        if method.output_count > 1 {
            if !result_names.insert(result_name.clone()) {
                return Err(format!(
                    "{} cannot be implemented: result type name collision {result_name}",
                    plan.name
                ));
            }
            declarations.push_str(&format!("\nclass {result_name}(TypedDict):\n"));
            let types = method
                .parameters
                .iter()
                .filter(|p| p.output_index.is_some())
                .map(|p| &p.typ)
                .chain(method.return_type.iter());
            for (name, typ) in output_names(method)?.iter().zip(types) {
                declarations.push_str(&format!(
                    "    {name}: {}\n",
                    projector.annotation(typ, true)
                ));
            }
        }
    }
    for (index, typ) in projector.structs.iter().enumerate() {
        support.push_str(&format!(
            "\ndef _{}Struct{index}(value, label):\n",
            projector.prefix
        ));
        if let Some(kind) = foundation_type(&typ.metadata) {
            let TypeMeta::Struct { name, .. } = &typ.metadata else {
                unreachable!()
            };
            let annotation = match kind {
                FoundationType::DateTime => "datetime",
                FoundationType::TimeSpan => "timedelta",
            };
            support.push_str(&format!(
                "    _implementation_check(value, isinstance(value, {annotation}), label)\n    return _pack_{}(value).to_value()\n\n",
                to_snake_case(name)
            ));
            continue;
        }
        support.push_str(&format!(
            "    _implementation_check(value, isinstance(value, {}), label)\n    s = DynWinRTStruct.create({})\n",
            projector.struct_symbol(&typ.metadata), projector.native(typ)
        ));
        let (
            ImplementationAbi::Struct(fields),
            TypeMeta::Struct {
                fields: metadata, ..
            },
        ) = (&typ.abi, &typ.metadata)
        else {
            unreachable!()
        };
        for (index, (field, typ)) in metadata.iter().zip(fields).enumerate() {
            let field_name = to_snake_case(&field.name);
            let value = format!("value.{field_name}");
            let setter = match &typ.abi {
                ImplementationAbi::Reference => format!(
                    "s.set_object({index}, {})",
                    projector.write(typ, &value, &field_name, None)
                ),
                ImplementationAbi::Struct(_) => format!(
                    "s.set_struct({index}, {}.as_struct())",
                    projector.write(typ, &value, &field_name, None)
                ),
                _ => super::structs::py_struct_field_setter(
                    &typ.metadata,
                    index,
                    &projector.check_scalar(typ, &value, &field_name),
                ),
            };
            support.push_str(&format!("    {setter}\n"));
        }
        support.push_str("    return s.to_value()\n\n");
    }
    for (index, delegate) in plan.delegates.iter().enumerate() {
        let method = &delegate.invoke;
        let params = projector.parameters(method, true)?;
        let comma = if params.is_empty() { "" } else { ", " };
        declarations.push_str(&format!(
            "\nclass {}Delegate{index}(Protocol):\n    @property\n    def _obj(self) -> DynWinRTValue: ...\n    def __call__(self{comma}{params}, /) -> {}: ...\n",
            projector.prefix, if method.output_count > 1 { format!("{}Delegate{index}Result", projector.prefix) } else { projector.result_type(method, false) }
        ));
        let args = method
            .parameters
            .iter()
            .filter_map(|parameter| {
                parameter.input_index.map(|index| {
                    if parameter.direction == ParamDirection::OutFill {
                        projector.delegate_fill_input(&parameter.typ, &format!("args[{index}]"))
                    } else {
                        projector.write(
                            &parameter.typ,
                            &format!("args[{index}]"),
                            "delegate argument",
                            None,
                        )
                    }
                })
            })
            .collect::<Vec<_>>()
            .join(", ");
        let outputs = method
            .parameters
            .iter()
            .filter(|p| p.output_index.is_some())
            .map(|p| &p.typ)
            .chain(method.return_type.iter())
            .enumerate()
            .map(|(index, typ)| projector.read(typ, &format!("result[{index}]")))
            .collect::<Vec<_>>();
        let returned = match outputs.as_slice() {
            [] => "None".into(),
            [value] => value.clone(),
            _ => format!(
                "{{ {} }}",
                output_names(method)?
                    .iter()
                    .zip(outputs)
                    .map(|(name, expr)| format!("{}: {expr}", quote(name)))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        support.push_str(&format!(
            "\ndef _{}Delegate{index}(value):\n    if value.is_null():\n        return None\n    def callback(*args):\n        if len(args) != {}:\n            raise TypeError('delegate argument count mismatch')\n        result = value.invoke_delegate({}, {}, [{args}])\n        return {returned}\n    callback._obj = value\n    return callback\n\n",
            projector.prefix, method.input_count, projector.iid(&delegate.typ), projector.signature(method)
        ));
    }
    declarations.push_str(&format!(
        "\nclass {}Handlers(Protocol):\n    \"\"\"Synchronous handlers; multi-output results are named dicts and FillArray inputs are capacities.\"\"\"\n", iface.name
    ));
    let mut handlers = HashSet::new();
    let mut bindings = Vec::new();
    let mut dispatch = String::new();
    let mut descriptors = Vec::new();
    for method in &plan.methods {
        let handler = handler_name(method);
        if !handlers.insert(handler.clone()) {
            return Err(format!(
                "{} cannot be implemented: handler name collision for {handler}",
                plan.name
            ));
        }
        let params = projector.parameters(method, false)?;
        declarations.push_str(&format!(
            "    def {handler}(self{}{}) -> {}: ...\n",
            if params.is_empty() { "" } else { ", " },
            params,
            projector.result_type(method, true)
        ));
        let slot = method.vtable_index;
        bindings.push(format!(
            "        h{slot} = _implementation_handler(handlers, {}, {}, {})",
            quote(&handler),
            method.input_count,
            quote(&plan.name)
        ));
        descriptors.push(format!(
            "DynWinRTImplementationMethod({}, {slot}, {})",
            quote(&method.name),
            projector.signature(method)
        ));
        let args = method
            .parameters
            .iter()
            .filter_map(|parameter| {
                parameter.input_index.map(|index| {
                    if parameter.direction == ParamDirection::OutFill {
                        format!("args[{index}].to_u32()")
                    } else {
                        projector.read(&parameter.typ, &format!("args[{index}]"))
                    }
                })
            })
            .collect::<Vec<_>>()
            .join(", ");
        let label = format!("{}.{}", plan.name, handler);
        dispatch.push_str(&format!(
            "            if vtable_index == {slot}:\n                if len(args) != {}:\n                    raise TypeError('implementation argument count mismatch')\n                result = _implementation_sync(h{slot}({args}), {})\n", method.input_count, quote(&label)
        ));
        let names = output_names(method)?;
        let mut outputs = Vec::new();
        for (index, (typ, fill)) in method
            .parameters
            .iter()
            .filter(|p| p.output_index.is_some())
            .map(|p| {
                (
                    &p.typ,
                    if p.direction == ParamDirection::OutFill {
                        p.input_index
                    } else {
                        None
                    },
                )
            })
            .chain(method.return_type.iter().map(|typ| (typ, None)))
            .enumerate()
        {
            let value = if method.output_count == 1 {
                "result".into()
            } else {
                format!(
                    "_implementation_field(result, {}, {})",
                    quote(&names[index]),
                    quote(&label)
                )
            };
            let capacity = fill.map(|index| format!("args[{index}].to_u32()"));
            outputs.push(projector.write(
                typ,
                &value,
                &format!("{label}.{}", names[index]),
                capacity.as_deref(),
            ));
        }
        if outputs.is_empty() {
            dispatch.push_str(&format!(
                "                if result is not None:\n                    raise TypeError({})\n",
                quote(&format!("{label}: void handler must return None"))
            ));
        }
        dispatch.push_str(&format!(
            "                return [{}]\n",
            outputs.join(", ")
        ));
    }
    support.push_str(&format!(
        "\n_implementation_plan = None\n\ndef _get_implementation_plan():\n    global _implementation_plan\n    if _implementation_plan is None:\n        _implementation_plan = DynWinRTInterfacePlan.create({}, DynWinRTType.interface(WinGUID.parse({})), [\n            {}\n        ], [{}])\n    return _implementation_plan\n\n",
        quote(&plan.name), quote(&plan.iid), descriptors.join(",\n            "),
        plan.required_interfaces.iter().map(|typ| projector.iid(typ)).collect::<Vec<_>>().join(", ")
    ));
    if !plan.delegates.is_empty() {
        bindings.push("        if not callable(getattr(DynWinRTValue, 'invoke_delegate', None)):\n            raise TypeError('this interface requires WinRT delegate invocation support in the runtime')".into());
    }
    let mut factory_body = format!(
        "    @staticmethod\n    def implementation(handlers: {}Handlers) -> DynWinRTImplementationDescriptor:\n{}\n        plan = _get_implementation_plan()\n        def dispatch(vtable_index, args):\n            if not isinstance(args, list):\n                raise TypeError('implementation arguments must be a list')\n{dispatch}            raise ValueError('unknown implementation vtable slot')\n        return DynWinRTImplementationDescriptor(plan, dispatch)\n\n    @classmethod\n    def implement(cls, handlers: {}Handlers, *additional: DynWinRTImplementationDescriptor, interfaces=()) -> DynWinRTImplementationHandle:\n        return DynWinRTImplementationHandle._create(cls, handlers, additional, interfaces)\n\n",
        iface.name,
        bindings.join("\n"),
        iface.name
    );
    factory_body.push_str(&format!(
        "    @classmethod\n    def from_implementation(cls, owner: DynWinRTImplementation | DynWinRTImplementationHandle) -> '{}':\n        \"\"\"Query an independently owned view, tracked by the current lifetime scope.\"\"\"\n        if not isinstance(owner, (DynWinRTImplementation, DynWinRTImplementationHandle)):\n            raise TypeError('from_implementation requires a DynWinRTImplementation controller or handle')\n        value = owner.to_value()\n        try:\n            native = value.cast(IID_{})\n            try:\n                view = object.__new__(cls)\n                cls._set_native(view, native, cache=False)\n                return view\n            except BaseException:\n                native.release()\n                raise\n        finally:\n            value.release()\n\n",
        iface.name, iface.name
    ));
    let requirements = if plan.required_interfaces.is_empty() {
        String::new()
    } else {
        format!(
            "    # Required QI views must be supplied as additional descriptors: {}.\n",
            plan.required_interfaces
                .iter()
                .map(|typ| typ
                    .metadata
                    .type_identity()
                    .definition_name()
                    .unwrap_or("interface")
                    .to_string())
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    let package_overload = if context.is_packaged() {
        format!(
            "    @overload\n    def implement(cls, handlers: {name}Handlers, *additional: DynWinRTImplementationDescriptor, interfaces: Sequence[_PackageImplementationPair]) -> DynWinRTImplementationHandle[{name}]: ...\n    @overload\n",
            name = iface.name
        )
    } else {
        String::new()
    };
    let factory_declarations = format!(
        "{requirements}    def implementation(cls, handlers: {name}Handlers) -> DynWinRTImplementationDescriptor: ...\n{package_overload}    def implement(cls, handlers: {name}Handlers, *additional: DynWinRTImplementationDescriptor, interfaces: Sequence[tuple[_DynWinRTImplementationFactory[_ImplementationHandlers], _ImplementationHandlers]] = ...) -> DynWinRTImplementationHandle[{name}]: ...\n    def from_implementation(cls, owner: DynWinRTImplementation | DynWinRTImplementationHandle[object]) -> {name}: ...\n\n",
        name = iface.name
    );
    let mut exports = vec![format!("{}Handlers", iface.name)];
    exports.extend(
        (0..plan.delegates.len()).map(|index| format!("{}Delegate{index}", projector.prefix)),
    );
    exports.extend(result_names);
    exports.sort();
    Ok(ImplementationProjection {
        support_code: support,
        declarations,
        factory_body,
        factory_declarations,
        supported: true,
        exports,
    })
}

pub(super) fn exports(context: &PythonProjectionContext, iface: &InterfaceMeta) -> Vec<String> {
    let mut projected = iface.clone();
    projected.name = context.projected_name_for_interface(iface);
    project(context, &projected).exports
}
