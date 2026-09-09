// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! JavaScript projection of validated reverse-call contracts.

use std::collections::HashSet;

use crate::codegen::winrt::shared::implementation::{
    ImplementationAbi, ImplementationHandlerKind, ImplementationMethod, ImplementationProjection,
    ImplementationType, WinRtImplementationPlan, project_implementation, validate_type,
};
use crate::codegen::winrt::shared::imports::{NO_DEFERRED, ireference_inner_type};
use crate::meta::{InterfaceMeta, ParamDirection};
use crate::types::TypeMeta;

use super::JavaScriptProjectionContext;
use super::naming::{capitalize, to_camel_case};
use super::signature::{convert_return, ts_dynwinrt_type};

const HELPERS: &str = r#"
function __implementationCheck(value, valid, label) {
    if (!valid) throw new TypeError(`${label}: invalid implementation result`);
    return value;
}
function __implementationField(value, name, label) {
    if (value === null || typeof value !== 'object') throw new TypeError(`${label}: expected a named result object`);
    const field = Object.getOwnPropertyDescriptor(value, name);
    if (!field || !Object.hasOwn(field, 'value')) throw new TypeError(`${label}: missing data field ${name}`);
    return field.value;
}
function __implementationArray(value, label, bytes = false, capacity = undefined) {
    if (!Array.isArray(value) && !(bytes && value instanceof Uint8Array)) throw new TypeError(`${label}: expected an array`);
    if (capacity !== undefined && value.length !== capacity) throw new RangeError(`${label}: FillArray result must match capacity ${capacity}`);
    return Array.from(value);
}
function __implementationReference(value, iid, label) {
    if (value === null) return DynWinRtValue.nullValue();
    const raw = value instanceof DynWinRtValue ? value : value?._obj;
    if (!(raw instanceof DynWinRtValue)) throw new TypeError(`${label}: expected a managed WinRT value or null`);
    return raw.isNull() ? DynWinRtValue.nullValue() : raw.cast(iid);
}
function __implementationSync(value, label) {
    if (value !== null && (typeof value === 'object' || typeof value === 'function') && typeof value.then === 'function') {
        throw new TypeError(`${label}: implementation callbacks must return synchronously, not a Promise or thenable`);
    }
    return value;
}
function __implementationHandler(handlers, name, label) {
    if (handlers === null || (typeof handlers !== 'object' && typeof handlers !== 'function')) throw new TypeError(`${label}: expected a handler object`);
    let owner = handlers;
    while (owner && owner !== Object.prototype && owner !== Function.prototype) {
        const property = Object.getOwnPropertyDescriptor(owner, name);
        if (property) {
            const callback = property.value;
            if (typeof callback !== 'function') throw new TypeError(`${label}: missing synchronous handler ${name} (accessors are not handlers)`);
            if (Object.prototype.toString.call(callback) === '[object AsyncFunction]' || Object.prototype.toString.call(callback) === '[object AsyncGeneratorFunction]') {
                throw new TypeError(`${label}.${name}: async handlers are not supported`);
            }
            return callback.bind(handlers);
        }
        owner = Object.getPrototypeOf(owner);
    }
    throw new TypeError(`${label}: missing synchronous handler ${name}`);
}
"#;

fn quote(value: &str) -> String {
    serde_json::to_string(value).expect("string serialization")
}

fn handler_name(method: &ImplementationMethod) -> String {
    let (prefix, raw) = match method.handler_kind {
        ImplementationHandlerKind::Method => return to_camel_case(&method.name),
        ImplementationHandlerKind::Getter => ("get", "get_"),
        ImplementationHandlerKind::Setter => ("set", "put_"),
        ImplementationHandlerKind::Add => ("add", "add_"),
        ImplementationHandlerKind::Remove => ("remove", "remove_"),
    };
    format!(
        "{prefix}{}",
        capitalize(method.name.strip_prefix(raw).unwrap_or(&method.name))
    )
}

fn input_name(name: &str, index: usize, fill: bool) -> String {
    let name = if name.is_empty() {
        format!("arg{index}")
    } else {
        to_camel_case(name)
    };
    if fill {
        format!("{name}Capacity")
    } else {
        name
    }
}

fn output_names(method: &ImplementationMethod) -> Result<Vec<String>, String> {
    let names = method
        .output_names
        .iter()
        .map(|name| to_camel_case(name))
        .collect::<Vec<_>>();
    if names.iter().collect::<HashSet<_>>().len() != names.len() {
        return Err(format!("{} has colliding output names", method.name));
    }
    Ok(names)
}

struct Projector<'a> {
    context: &'a JavaScriptProjectionContext,
    known: &'a HashSet<String>,
    plan: &'a WinRtImplementationPlan,
    prefix: String,
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
                format!("DynWinRtType.arrayType({})", self.native(element))
            }
            ImplementationAbi::Struct(fields) => {
                let TypeMeta::Struct {
                    namespace, name, ..
                } = &typ.metadata
                else {
                    unreachable!()
                };
                format!(
                    "DynWinRtType.structType({}, [{}])",
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
                    format!("DynWinRtType.{kind}(WinGuid.parse({}))", quote(iid))
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
                        "DynWinRtType.runtimeClass({}, {})",
                        quote(&format!(
                            "{namespace}.{}",
                            self.context.metadata_type_name(namespace, name)
                        )),
                        self.native(&default)
                    )
                }
                TypeMeta::Parameterized { piid, args, .. } => format!(
                    "DynWinRtType.parameterized(WinGuid.parse({}), [{}])",
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
                | TypeMeta::AsyncOperationWithProgress(_, _) => {
                    ts_dynwinrt_type(self.context, &typ.metadata)
                }
                _ => unreachable!("validated reference"),
            },
            ImplementationAbi::Scalar if typ.metadata == TypeMeta::Char16 => {
                "DynWinRtType.char16()".into()
            }
            ImplementationAbi::Scalar
            | ImplementationAbi::HResult
            | ImplementationAbi::Enum
            | ImplementationAbi::Guid
            | ImplementationAbi::HString => ts_dynwinrt_type(self.context, &typ.metadata),
        }
    }

    fn iid(&self, typ: &ImplementationType) -> String {
        if typ.metadata == TypeMeta::Object {
            "WinGuid.parse('af86e2e0-b12d-4c6a-9c5a-d7aa65101e90')".into()
        } else {
            format!("{}.iid()", self.native(typ))
        }
    }

    fn annotation(&self, typ: &ImplementationType, writing: bool) -> String {
        if let Some(index) = self.delegate_index(&typ.metadata) {
            return format!(
                "{}Delegate{index}{} | null",
                self.prefix,
                if writing { " | DynWinRtValue" } else { "" }
            );
        }
        if let Some(inner) = ireference_inner_type(&typ.metadata) {
            return format!(
                "{} | null",
                self.annotation(
                    &validate_type(inner, false).expect("validated IReference"),
                    writing
                )
            );
        }
        match &typ.abi {
            ImplementationAbi::Array(element) => {
                let array = format!("({})[]", self.annotation(element, writing));
                if writing && element.metadata == TypeMeta::U8 {
                    format!("{array} | Uint8Array")
                } else {
                    array
                }
            }
            ImplementationAbi::Struct(fields) => {
                let TypeMeta::Struct {
                    fields: metadata, ..
                } = &typ.metadata
                else {
                    unreachable!()
                };
                format!(
                    "{{ {} }}",
                    metadata
                        .iter()
                        .zip(fields)
                        .map(|(field, typ)| {
                            format!(
                                "{}: {}",
                                to_camel_case(&field.name),
                                self.annotation(typ, writing)
                            )
                        })
                        .collect::<Vec<_>>()
                        .join("; ")
                )
            }
            ImplementationAbi::HResult => "number".into(),
            ImplementationAbi::Enum => match &typ.metadata {
                TypeMeta::Enum { name, .. } if self.known.contains(name) => name.clone(),
                _ => "number".into(),
            },
            ImplementationAbi::Reference => {
                let name = match &typ.metadata {
                    TypeMeta::RuntimeClass { name, .. } | TypeMeta::Interface { name, .. }
                        if self.known.contains(name) =>
                    {
                        name.clone()
                    }
                    TypeMeta::Parameterized {
                        namespace,
                        name,
                        piid,
                        args,
                    } => {
                        let concrete = self
                            .context
                            .projected_parameterized_name(namespace, name, piid, args);
                        if self.known.contains(&concrete) {
                            concrete
                        } else {
                            "DynWinRtValue".into()
                        }
                    }
                    _ => "DynWinRtValue".into(),
                };
                format!("{name} | null")
            }
            ImplementationAbi::Scalar | ImplementationAbi::Guid | ImplementationAbi::HString => {
                match typ.metadata {
                    TypeMeta::Bool => "boolean",
                    TypeMeta::I64 | TypeMeta::U64 => "bigint",
                    TypeMeta::String | TypeMeta::Guid => "string",
                    _ => "number",
                }
                .into()
            }
        }
    }

    fn read(&self, typ: &ImplementationType, value: &str) -> String {
        if let Some(index) = self.delegate_index(&typ.metadata) {
            return format!("{}Delegate{index}({value})", self.prefix);
        }
        if let Some(inner) = ireference_inner_type(&typ.metadata) {
            let inner = validate_type(inner, false).expect("validated IReference value");
            let native = format!(
                "DynWinRtType.registerInterface('__dynwinrt_reverse_reference_' + iid.toString(), iid).addMethod('get_Value', new DynWinRtMethodSig().addOut({})).method(6).invoke(v.cast(iid), [])",
                self.native(&inner)
            );
            return format!(
                "((v) => {{ if (v.isNull()) return null; const iid = {}; return {}; }})({value})",
                self.iid(typ),
                self.read(&inner, &native)
            );
        }
        match &typ.abi {
            ImplementationAbi::Array(element) => format!(
                "{value}.asArray().toValues().map(v => {})",
                self.read(element, "v")
            ),
            ImplementationAbi::Struct(fields) => {
                let TypeMeta::Struct {
                    fields: metadata, ..
                } = &typ.metadata
                else {
                    unreachable!()
                };
                let members = metadata
                    .iter()
                    .zip(fields)
                    .enumerate()
                    .map(|(index, (field, typ))| {
                        let expr = match &typ.abi {
                            ImplementationAbi::Reference => {
                                self.read(typ, &format!("s.getObject({index})"))
                            }
                            ImplementationAbi::Struct(_) => {
                                self.read(typ, &format!("s.getStruct({index}).toValue()"))
                            }
                            _ => super::structs::struct_field_getter(
                                self.context,
                                &typ.metadata,
                                index,
                            ),
                        };
                        format!("{}: {expr}", to_camel_case(&field.name))
                    })
                    .collect::<Vec<_>>()
                    .join(", ");
                format!("((s) => ({{ {members} }}))({value}.asStruct())")
            }
            ImplementationAbi::Reference if typ.metadata.is_async() => {
                format!("((v) => v.isNull() ? null : v)({value})")
            }
            _ => convert_return(
                self.context,
                value,
                Some(&typ.metadata),
                false,
                self.known,
                &NO_DEFERRED,
            ),
        }
    }

    fn check_scalar(&self, typ: &ImplementationType, value: &str, label: &str) -> String {
        let integer = |min: &str, max: &str| {
            format!("typeof v === 'number' && Number.isInteger(v) && v >= {min} && v <= {max}")
        };
        let check = match &typ.metadata {
            TypeMeta::Bool => "typeof v === 'boolean'".into(),
            TypeMeta::String | TypeMeta::Guid => "typeof v === 'string'".into(),
            TypeMeta::I8 => integer("-128", "127"),
            TypeMeta::U8 => integer("0", "255"),
            TypeMeta::I16 => integer("-32768", "32767"),
            TypeMeta::U16 | TypeMeta::Char16 => integer("0", "65535"),
            TypeMeta::I32 | TypeMeta::Enum { .. } | TypeMeta::Struct { .. } => integer("-2147483648", "2147483647"),
            TypeMeta::U32 => integer("0", "4294967295"),
            TypeMeta::I64 => "typeof v === 'bigint' && v >= -9223372036854775808n && v <= 9223372036854775807n".into(),
            TypeMeta::U64 => "typeof v === 'bigint' && v >= 0n && v <= 18446744073709551615n".into(),
            TypeMeta::F32 => "typeof v === 'number' && (!Number.isFinite(v) || Math.abs(v) <= 3.4028234663852886e38)".into(),
            TypeMeta::F64 => "typeof v === 'number'".into(),
            _ => unreachable!("validated scalar"),
        };
        format!(
            "((v) => __implementationCheck(v, {check}, {}))({value})",
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
                "((v) => v === null ? DynWinRtValue.nullValue() : (v instanceof DynWinRtValue || v?._obj instanceof DynWinRtValue) ? __implementationReference(v, {}, {}) : DynWinRtValue.boxReference({}, {}))({value})",
                self.iid(typ),
                quote(label),
                self.write(&inner, "v", label, None),
                self.native(&inner)
            );
        }
        match &typ.abi {
            ImplementationAbi::Reference => format!(
                "__implementationReference({value}, {}, {})",
                self.iid(typ),
                quote(label)
            ),
            ImplementationAbi::Array(element)
                if matches!(element.abi, ImplementationAbi::HResult) =>
            {
                format!(
                    "DynWinRtArray.fromHresultValues(__implementationArray({value}, {}, false, {}).map(v => {})).toValue()",
                    quote(label),
                    capacity.unwrap_or("undefined"),
                    self.check_scalar(element, "v", &format!("{label}[]"))
                )
            }
            ImplementationAbi::Array(element) => format!(
                "DynWinRtArray.fromObjectValues(__implementationArray({value}, {}, {}, {}).map(v => {}), {}).toValue()",
                quote(label),
                element.metadata == TypeMeta::U8,
                capacity.unwrap_or("undefined"),
                self.write(element, "v", &format!("{label}[]"), None),
                self.native(element)
            ),
            ImplementationAbi::Struct(fields) => {
                let TypeMeta::Struct {
                    fields: metadata, ..
                } = &typ.metadata
                else {
                    unreachable!()
                };
                let setters = metadata
                    .iter()
                    .zip(fields)
                    .enumerate()
                    .map(|(index, (field, typ))| {
                        let field_name = to_camel_case(&field.name);
                        let label = format!("{label}.{field_name}");
                        let access = format!(
                            "__implementationField(v, {}, {})",
                            quote(&field_name),
                            quote(&label)
                        );
                        let setter = match &typ.abi {
                            ImplementationAbi::Reference => format!(
                                "s.setObject({index}, {})",
                                self.write(typ, &access, &label, None)
                            ),
                            ImplementationAbi::Struct(_) => format!(
                                "s.setStruct({index}, {}.asStruct())",
                                self.write(typ, &access, &label, None)
                            ),
                            _ => super::structs::struct_field_setter(
                                self.context,
                                &typ.metadata,
                                index,
                                &self.check_scalar(typ, &access, &label),
                            ),
                        };
                        format!("{setter};")
                    })
                    .collect::<Vec<_>>()
                    .join(" ");
                format!(
                    "((v) => {{ const s = DynWinRtStruct.create({}); {setters} return s.toValue(); }})({value})",
                    self.native(typ)
                )
            }
            ImplementationAbi::HResult => format!(
                "DynWinRtValue.hresult({})",
                self.check_scalar(typ, value, label)
            ),
            ImplementationAbi::Scalar
            | ImplementationAbi::Enum
            | ImplementationAbi::Guid
            | ImplementationAbi::HString => {
                let value = self.check_scalar(typ, value, label);
                let constructor = match typ.metadata {
                    TypeMeta::Bool => "boolValue",
                    TypeMeta::I8 => "i8Value",
                    TypeMeta::U8 => "u8Value",
                    TypeMeta::I16 => "i16",
                    TypeMeta::U16 | TypeMeta::Char16 => "u16",
                    TypeMeta::I32 => "i32",
                    TypeMeta::U32 => "u32",
                    TypeMeta::I64 => "i64",
                    TypeMeta::U64 => "u64",
                    TypeMeta::F32 => "f32",
                    TypeMeta::F64 => "f64",
                    TypeMeta::String => "hstring",
                    TypeMeta::Guid => return format!("DynWinRtValue.guid(WinGuid.parse({value}))"),
                    TypeMeta::Enum { .. } => {
                        return format!("DynWinRtValue.enumValue({}, {value})", self.native(typ));
                    }
                    _ => unreachable!("validated scalar"),
                };
                format!("DynWinRtValue.{constructor}({value})")
            }
        }
    }

    fn signature(&self, method: &ImplementationMethod) -> String {
        let mut signature = "new DynWinRtMethodSig()".to_string();
        for parameter in &method.parameters {
            let add = match parameter.direction {
                ParamDirection::In => "addIn",
                ParamDirection::Out => "addOut",
                ParamDirection::OutFill => "addOutFill",
            };
            signature.push_str(&format!(".{add}({})", self.native(&parameter.typ)));
        }
        if let Some(typ) = &method.return_type {
            signature.push_str(&format!(".addOut({})", self.native(typ)));
        }
        signature
    }

    fn default_value(&self, typ: &ImplementationType) -> String {
        match &typ.abi {
            ImplementationAbi::Reference => "DynWinRtValue.nullValue()".into(),
            ImplementationAbi::Struct(_) => {
                format!("DynWinRtStruct.create({}).toValue()", self.native(typ))
            }
            ImplementationAbi::Array(_) => unreachable!("nested arrays were rejected"),
            ImplementationAbi::Enum => format!("DynWinRtValue.enumValue({}, 0)", self.native(typ)),
            _ => self.write(
                typ,
                match typ.metadata {
                    TypeMeta::Bool => "false",
                    TypeMeta::I64 | TypeMeta::U64 => "0n",
                    TypeMeta::String => "''",
                    TypeMeta::Guid => "'00000000-0000-0000-0000-000000000000'",
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
            "DynWinRtArray.fromObjectValues(Array.from({{ length: __implementationCheck({capacity}, Number.isInteger({capacity}) && {capacity} >= 0 && {capacity} <= 4294967295, 'delegate capacity') }}, () => {}), {}).toValue()",
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
                let name = input_name(&parameter.name, index, fill);
                if !names.insert(name.clone()) {
                    return Err(format!(
                        "{} has colliding handler parameter names",
                        method.name
                    ));
                }
                params.push(format!(
                    "{name}: {}",
                    if fill {
                        "number".into()
                    } else {
                        self.annotation(&parameter.typ, writing)
                    }
                ));
            }
        }
        Ok(params.join(", "))
    }

    fn result_type(&self, method: &ImplementationMethod, writing: bool) -> Result<String, String> {
        let types = method
            .parameters
            .iter()
            .filter(|p| p.output_index.is_some())
            .map(|p| &p.typ)
            .chain(method.return_type.iter())
            .collect::<Vec<_>>();
        Ok(match types.as_slice() {
            [] => "void".into(),
            [typ] => self.annotation(typ, writing),
            _ => format!(
                "{{ {} }}",
                output_names(method)?
                    .iter()
                    .zip(types)
                    .map(|(name, typ)| { format!("{name}: {}", self.annotation(typ, writing)) })
                    .collect::<Vec<_>>()
                    .join("; ")
            ),
        })
    }
}

pub(super) fn project(
    context: &JavaScriptProjectionContext,
    iface: &InterfaceMeta,
    known: &HashSet<String>,
) -> ImplementationProjection {
    let result = project_implementation(iface)
        .and_then(|plan| project_validated(context, iface, known, &plan));
    result.unwrap_or_else(|reason| ImplementationProjection {
        factory_body: format!(
            "    static implementation() {{ throw new TypeError({0}); }}\n    static implement() {{ throw new TypeError({0}); }}\n", quote(&reason)
        ),
        factory_declarations: format!(
            "    /** {} */\n    static implementation(handlers: never): never;\n    /** {} */\n    static implement(handlers: never, ...additional: never[]): never;\n",
            reason.replace("*/", "* /"), reason.replace("*/", "* /")
        ),
        ..Default::default()
    })
}

fn project_validated(
    context: &JavaScriptProjectionContext,
    iface: &InterfaceMeta,
    known: &HashSet<String>,
    plan: &WinRtImplementationPlan,
) -> Result<ImplementationProjection, String> {
    let projector = Projector {
        context,
        known,
        plan,
        prefix: format!("{}Implementation", iface.name),
    };
    let mut support = String::from(HELPERS);
    let mut declarations = String::new();
    for (index, delegate) in plan.delegates.iter().enumerate() {
        let method = &delegate.invoke;
        declarations.push_str(&format!(
            "export type {}Delegate{index} = (({}) => {}) & {{ readonly _obj: DynWinRtValue }};\n",
            projector.prefix,
            projector.parameters(method, true)?,
            projector.result_type(method, false)?
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
            [] => "undefined".into(),
            [value] => value.clone(),
            _ => format!(
                "{{ {} }}",
                output_names(method)?
                    .iter()
                    .zip(outputs)
                    .map(|(name, expr)| format!("{name}: {expr}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        };
        support.push_str(&format!(
            "\nfunction {}Delegate{index}(value) {{\n    if (value.isNull()) return null;\n    const callback = (...args) => {{\n        if (args.length !== {}) throw new TypeError('delegate argument count mismatch');\n        const result = value.invokeDelegate({}, {}, [{args}]);\n        return {returned};\n    }};\n    Object.defineProperty(callback, '_obj', {{ value }});\n    return callback;\n}}\n",
            projector.prefix, method.input_count, projector.iid(&delegate.typ), projector.signature(method)
        ));
    }
    declarations.push_str(&format!(
        "\n/** Synchronous owner-thread handlers. Out parameters precede the logical result; FillArray inputs are capacities. */\nexport interface {}Handlers {{\n", iface.name
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
        declarations.push_str(&format!(
            "    {handler}({}): {};\n",
            projector.parameters(method, false)?,
            projector.result_type(method, true)?
        ));
        let slot = method.vtable_index;
        bindings.push(format!(
            "        const h{slot} = __implementationHandler(handlers, {}, {});",
            quote(&handler),
            quote(&plan.name)
        ));
        descriptors.push(format!(
            "{{ name: {}, vtableIndex: {slot}, signature: {} }}",
            quote(&method.name),
            projector.signature(method)
        ));
        let args = method
            .parameters
            .iter()
            .filter_map(|parameter| {
                parameter.input_index.map(|index| {
                    if parameter.direction == ParamDirection::OutFill {
                        format!("args[{index}].toNumber()")
                    } else {
                        projector.read(&parameter.typ, &format!("args[{index}]"))
                    }
                })
            })
            .collect::<Vec<_>>()
            .join(", ");
        let label = format!("{}.{}", plan.name, handler);
        dispatch.push_str(&format!(
            "                case {slot}: {{\n                    if (args.length !== {}) throw new TypeError('implementation argument count mismatch');\n                    const result = __implementationSync(h{slot}({args}), {});\n", method.input_count, quote(&label)
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
                    "__implementationField(result, {}, {})",
                    quote(&names[index]),
                    quote(&label)
                )
            };
            let capacity = fill.map(|index| format!("args[{index}].toNumber()"));
            outputs.push(projector.write(
                typ,
                &value,
                &format!("{label}.{}", names[index]),
                capacity.as_deref(),
            ));
        }
        if outputs.is_empty() {
            dispatch.push_str(&format!(
                "                    if (result !== undefined) throw new TypeError({});\n",
                quote(&format!("{label}: void handler must return undefined"))
            ));
        }
        dispatch.push_str(&format!(
            "                    return [{}];\n                }}\n",
            outputs.join(", ")
        ));
    }
    declarations.push_str("}\n");
    support.push_str(&format!(
        "\nlet __implementationPlan;\nfunction __getImplementationPlan() {{\n    return (__implementationPlan ??= DynWinRtInterfacePlan.create({}, DynWinRtType.interface(WinGuid.parse({})), [\n        {}\n    ], [{}]));\n}}\n",
        quote(&plan.name), quote(&plan.iid), descriptors.join(",\n        "),
        plan.required_interfaces.iter().map(|typ| projector.iid(typ)).collect::<Vec<_>>().join(", ")
    ));
    let requirements = if plan.required_interfaces.is_empty() {
        String::new()
    } else {
        format!(
            " Required QI views must be supplied as additional descriptors: {}.",
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
    if !plan.delegates.is_empty() {
        bindings.push("        if (typeof DynWinRtValue.prototype.invokeDelegate !== 'function') throw new TypeError('this interface requires WinRT delegate invocation support in the runtime');".into());
    }
    let mut factory_body = format!(
        "    static implementation(handlers) {{\n{}\n        const plan = __getImplementationPlan();\n        return Object.freeze({{ plan, dispatch(vtableIndex, args) {{\n            if (!Array.isArray(args)) throw new TypeError('implementation arguments must be an array');\n            switch (vtableIndex) {{\n{dispatch}                default: throw new RangeError('unknown implementation vtable slot');\n            }}\n        }} }});\n    }}\n\n    static implement(handlers, ...additional) {{\n        return DynWinRtImplementationHandle.create(this, handlers, additional, view => view._obj.release());\n    }}\n",
        bindings.join("\n")
    );
    factory_body.push_str(
        "\n    static fromImplementation(owner) {\n        if (!(owner instanceof DynWinRtImplementation) && !(owner instanceof DynWinRtImplementationHandle)) throw new TypeError('fromImplementation requires a DynWinRtImplementation controller or handle');\n        const value = owner.toValue();\n        try {\n            return this.from(value);\n        } finally {\n            value.release();\n        }\n    }\n",
    );
    let factory_declarations = format!(
        "    /** Validate synchronous handlers and create an independent interface descriptor.{requirements} */\n    static implementation(handlers: {name}Handlers): DynWinRtImplementationDescriptor;\n    /** Create an instance with a stable typed value and explicit lifetime management.{requirements} */\n    static implement<const T extends readonly DynWinRtImplementationType[]>(handlers: {name}Handlers, options: DynWinRtImplementationOptions<T>): DynWinRtImplementationHandle<{name}>;\n    static implement(handlers: {name}Handlers, ...additional: DynWinRtImplementationDescriptor[]): DynWinRtImplementationHandle<{name}>;\n    /** Query an independent view, without consuming the handle's primary value. */\n    static fromImplementation(owner: DynWinRtImplementation | DynWinRtImplementationHandle<unknown>): {name};\n",
        name = iface.name
    );
    let exports = std::iter::once(format!("{}Handlers", iface.name))
        .chain(
            (0..plan.delegates.len()).map(|index| format!("{}Delegate{index}", projector.prefix)),
        )
        .collect();
    Ok(ImplementationProjection {
        support_code: support,
        declarations,
        factory_body,
        factory_declarations,
        supported: true,
        exports,
    })
}
