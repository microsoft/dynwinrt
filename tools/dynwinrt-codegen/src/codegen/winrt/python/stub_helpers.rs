// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Rendering helpers for Python type stubs.

use crate::codegen::winrt::shared::imports::get_in_params;
use crate::meta::MethodMeta;
use crate::types::{FieldMeta, TypeIdentity, TypeIdentityKind, TypeKind, TypeMeta};

use super::naming::{PythonProjectionContext, to_snake_case};
use super::native_types::{FoundationType, foundation_type};
use super::structs::{py_struct_field_read_type, py_struct_field_type};
use super::type_helpers::{
    method_pydoc_with_indent, py_delegate_callable_type, py_factory_return_type,
    py_method_return_type, py_output_type, py_param_list, py_param_type_safe,
};
use crate::codegen::winrt::shared::imports::ireference_inner_type;

pub(super) fn format_py_type_import(
    context: &PythonProjectionContext,
    namespace: &str,
    name: &str,
    kind: TypeKind,
) -> String {
    let identity_kind = match kind {
        TypeKind::Class => TypeIdentityKind::Class,
        TypeKind::Enum => TypeIdentityKind::Enum,
        TypeKind::Interface => TypeIdentityKind::Interface,
    };
    let identity = TypeIdentity::named(identity_kind, namespace, name);
    let module = context.implementation_module(&identity);
    let projected_name = context.projected_name(&identity);
    let reference_name = context.reference_name(&identity);
    let imported = |name: &str, alias: &str| {
        if name == alias {
            name.to_string()
        } else {
            format!("{name} as {alias}")
        }
    };
    if kind == TypeKind::Interface {
        format!(
            "from .{module} import {}, {}  # noqa: F401\n",
            imported(
                &format!("IID_{projected_name}"),
                &format!("IID_{reference_name}")
            ),
            imported(&projected_name, &reference_name)
        )
    } else if kind == TypeKind::Class {
        format!(
            "from .{module} import {}, {}  # noqa: F401\n",
            imported(&projected_name, &reference_name),
            imported(
                &format!("{projected_name}Like"),
                &format!("{reference_name}Like")
            )
        )
    } else {
        format!(
            "from .{module} import {}  # noqa: F401\n",
            imported(&projected_name, &reference_name)
        )
    }
}

pub(super) fn py_struct_export_names(s: &TypeMeta) -> Vec<String> {
    match s {
        TypeMeta::Struct { name, .. } => {
            let snake = to_snake_case(name);
            let mut names = if foundation_type(s).is_none() {
                vec![name.clone()]
            } else {
                Vec::new()
            };
            names.extend([
                format!("{}_TYPE", name),
                format!("pack_{}", snake),
                format!("unpack_{}", snake),
            ]);
            names
        }
        _ => vec![],
    }
}

pub(super) fn generate_struct_stub_imports(
    context: &PythonProjectionContext,
    structs: &[TypeMeta],
) -> String {
    let mut imports = structs
        .iter()
        .filter_map(|typ| {
            let TypeMeta::Struct { .. } = typ else {
                return None;
            };
            Some(format!(
                "from .{} import {}  # noqa: F401\n",
                context.implementation_module_for_type(typ),
                py_struct_export_names(typ).join(", ")
            ))
        })
        .collect::<Vec<_>>();
    imports.sort();
    imports.dedup();
    imports.concat()
}

pub(super) fn emit_struct_stub(context: &PythonProjectionContext, s: &TypeMeta) -> String {
    if let Some(kind) = foundation_type(s) {
        let TypeMeta::Struct { name, .. } = s else {
            unreachable!()
        };
        let snake_name = to_snake_case(name);
        let native_type = match kind {
            FoundationType::DateTime => "datetime",
            FoundationType::TimeSpan => "timedelta",
        };
        return format!(
            "\ndef unpack_{snake_name}(v: DynWinRTValue) -> {native_type}: ...\n\
             {name}_TYPE: 'DynWinRTType'\n\
             def pack_{snake_name}(v: {native_type}) -> DynWinRTStruct: ...\n"
        );
    }

    let (_namespace, name, fields) = match s {
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => (namespace, name, fields),
        _ => return String::new(),
    };
    let mut out = String::new();
    let snake_name = to_snake_case(name);
    let slot_names = fields.iter().map(py_struct_slot_name).collect::<Vec<_>>();

    out.push_str(&format!("\nclass {}:\n", name));
    out.push_str(&format!(
        "    __slots__ = {}\n",
        py_string_tuple_literal(&slot_names)
    ));
    if fields.is_empty() {
        out.push_str("    def __init__(self) -> None: ...\n");
    } else {
        let init_params: Vec<String> = fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {} = ...",
                    to_snake_case(&f.name),
                    py_struct_field_stub_type(context, &f.typ)
                )
            })
            .collect();
        out.push_str(&format!(
            "    def __init__(self, {}) -> None: ...\n",
            init_params.join(", ")
        ));
        for f in fields {
            let snake = to_snake_case(&f.name);
            if ireference_inner_type(&f.typ).is_some() {
                out.push_str(&format!(
                    "    @property\n\
                     \x20   def {snake}(self) -> {read_type}: ...\n\
                     \x20   @{snake}.setter\n\
                     \x20   def {snake}(self, value: {write_type}) -> None: ...\n",
                    read_type = py_struct_field_read_type(context, &f.typ),
                    write_type = py_struct_field_type(context, &f.typ),
                ));
            } else {
                out.push_str(&format!(
                    "    {}: {}\n",
                    snake,
                    py_struct_field_stub_type(context, &f.typ)
                ));
            }
        }
    }
    out.push_str("    def __eq__(self, other: object) -> bool: ...\n");
    out.push_str("    def __repr__(self) -> str: ...\n");
    out.push('\n');

    out.push_str(&format!(
        "def unpack_{}(v: DynWinRTValue) -> {}: ...\n",
        snake_name, name
    ));
    out.push_str(&format!("{}_TYPE: 'DynWinRTType'\n", name));
    out.push_str(&format!(
        "def pack_{}(v: {}) -> DynWinRTStruct: ...\n",
        snake_name, name
    ));
    out
}

fn py_struct_slot_name(field: &FieldMeta) -> String {
    let snake = to_snake_case(&field.name);
    if ireference_inner_type(&field.typ).is_some() {
        format!("_{snake}")
    } else {
        snake
    }
}

fn py_string_tuple_literal(values: &[String]) -> String {
    match values {
        [] => "()".to_string(),
        [value] => format!("('{value}',)"),
        _ => format!(
            "({})",
            values
                .iter()
                .map(|value| format!("'{value}'"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

pub(super) fn py_struct_field_stub_type(
    context: &PythonProjectionContext,
    typ: &TypeMeta,
) -> String {
    if ireference_inner_type(typ).is_some() {
        return py_struct_field_type(context, typ);
    }

    match typ {
        TypeMeta::Bool => "bool".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64 => "int".to_string(),
        TypeMeta::Enum { .. } => format!("'{}'", context.reference_name_for_type(typ)),
        TypeMeta::Char16 => "str".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "float".to_string(),
        TypeMeta::String => "str".to_string(),
        TypeMeta::Guid => "UUID".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "int".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::DateTime) => "datetime".to_string(),
        typ if foundation_type(typ) == Some(FoundationType::TimeSpan) => "timedelta".to_string(),
        TypeMeta::Struct { name, .. } => format!("'{}'", name),
        _ => "object".to_string(),
    }
}

pub(super) fn emit_method_stub(
    method: &MethodMeta,
    context: &PythonProjectionContext,
    indent_spaces: usize,
    event_has_remove: bool,
    property_has_getter: bool,
    overrides_mutable_sequence: bool,
) -> String {
    emit_method_stub_named(
        method,
        context,
        indent_spaces,
        None,
        event_has_remove,
        property_has_getter,
        overrides_mutable_sequence,
    )
}

fn emit_documented_stub(out: &mut String, indent: &str, signature: &str, doc: &str, suffix: &str) {
    if doc.is_empty() {
        out.push_str(&format!("{indent}{signature}: ...{suffix}\n"));
    } else {
        out.push_str(&format!("{indent}{signature}:{suffix}\n"));
        out.push_str(doc);
        out.push_str(&format!("{indent}    ...\n"));
    }
}

pub(super) fn emit_method_stub_named(
    method: &MethodMeta,
    context: &PythonProjectionContext,
    indent_spaces: usize,
    name_override: Option<&str>,
    event_has_remove: bool,
    property_has_getter: bool,
    overrides_mutable_sequence: bool,
) -> String {
    let indent = " ".repeat(indent_spaces);
    let in_params = get_in_params(method);
    let return_type = method.return_type.as_ref();

    let is_delegate_type =
        |typ: Option<&TypeMeta>| -> bool { typ.is_some_and(|typ| context.is_delegate_type(typ)) };

    let mut out = String::new();
    let body_indent = format!("{indent}    ");
    let doc = method_pydoc_with_indent(method, &in_params, &body_indent);

    // Events
    if method.is_event_add {
        let suffix = method.name.strip_prefix("add_").unwrap_or(&method.name);
        let event_name = to_snake_case(suffix);
        // Build a typed callback signature matching the runtime .py side.
        let delegate_typ = in_params.first().map(|p| &p.typ);
        let callback_sig = delegate_typ
            .map(|typ| py_delegate_callable_type(typ, context))
            .unwrap_or_else(|| "Callable[..., object]".to_string());
        emit_documented_stub(
            &mut out,
            &indent,
            &format!(
                "def on_{}(self, callback: {}) -> 'DynWinRTValue'",
                event_name, callback_sig
            ),
            &doc,
            "",
        );
        if event_has_remove {
            out.push_str(&format!(
                "{indent}def subscribe_{}(self, callback: {}) -> Callable[[], None]: ...\n",
                event_name, callback_sig
            ));
            out.push_str(&format!(
                "{indent}def once_{}(self, callback: {}) -> Callable[[], None]: ...\n",
                event_name, callback_sig
            ));
        }
        return out;
    }
    if method.is_event_remove {
        let event_name = to_snake_case(method.name.strip_prefix("remove_").unwrap_or(&method.name));
        emit_documented_stub(
            &mut out,
            &indent,
            &format!(
                "def off_{}(self, token: 'DynWinRTValue') -> None",
                event_name
            ),
            &doc,
            "",
        );
        return out;
    }

    if method.is_property_getter && in_params.is_empty() {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        let py_return = return_type
            .map(|typ| py_output_type(typ, context))
            .unwrap_or_else(|| "None".to_string());
        out.push_str(&format!("{indent}@builtins.property\n"));
        emit_documented_stub(
            &mut out,
            &indent,
            &format!("def {}(self) -> {}", prop_name, py_return),
            &doc,
            "",
        );
    } else if method.is_property_setter {
        let prop_name = to_snake_case(method.name.strip_prefix("put_").unwrap_or(&method.name));
        let param_type = if in_params
            .first()
            .is_some_and(|p| is_delegate_type(Some(&p.typ)))
        {
            "Callable[..., object] | 'DynWinRTValue'".to_string()
        } else {
            in_params
                .first()
                .map(|p| py_param_type_safe(&p.typ, context))
                .unwrap_or_else(|| "object".to_string())
        };
        if property_has_getter {
            out.push_str(&format!("{indent}@{}.setter\n", prop_name));
            emit_documented_stub(
                &mut out,
                &indent,
                &format!("def {}(self, value: {}) -> None", prop_name, param_type),
                &doc,
                "",
            );
        } else {
            emit_documented_stub(
                &mut out,
                &indent,
                &format!("def set_{}(self, value: {}) -> None", prop_name, param_type),
                &doc,
                "",
            );
        }
    } else {
        let py_params = py_param_list(&in_params, context);
        let py_return = py_method_return_type(method, context);
        let method_name = name_override
            .map(str::to_string)
            .unwrap_or_else(|| to_snake_case(&method.name));
        let self_and_params = if py_params.is_empty() {
            "self".to_string()
        } else {
            format!("self, {}", py_params)
        };
        // WinRT vectors may reject null on mutation while returning null
        // interface elements. That asymmetric native contract cannot satisfy
        // MutableSequence[T | None]'s append signature exactly. Empty
        // structural protocols can make mypy consider the override compatible.
        let override_ignore = if overrides_mutable_sequence
            && method_name == "append"
            && in_params.first().is_some_and(|param| {
                py_param_type_safe(&param.typ, context)
                    != super::type_helpers::py_return_type_safe(Some(&param.typ), context)
            }) {
            "  # type: ignore[override, unused-ignore]"
        } else {
            ""
        };
        emit_documented_stub(
            &mut out,
            &indent,
            &format!("def {}({}) -> {}", method_name, self_and_params, py_return),
            &doc,
            override_ignore,
        );
    }

    out
}

pub(super) fn emit_static_method_stub(
    class_name: &str,
    method: &MethodMeta,
    context: &PythonProjectionContext,
    is_factory: bool,
) -> String {
    emit_static_method_stub_named(class_name, method, context, is_factory, None)
}

pub(super) fn emit_static_method_stub_named(
    class_name: &str,
    method: &MethodMeta,
    context: &PythonProjectionContext,
    is_factory: bool,
    name_override: Option<&str>,
) -> String {
    let in_params = get_in_params(method);
    let py_params = py_param_list(&in_params, context);

    let py_return = if is_factory {
        py_factory_return_type(class_name, method, context)
    } else {
        py_method_return_type(method, context)
    };

    let mut out = String::new();
    let doc = method_pydoc_with_indent(method, &in_params, "        ");

    if is_factory || !method.is_property_getter || !in_params.is_empty() {
        let method_name = name_override
            .map(str::to_string)
            .unwrap_or_else(|| to_snake_case(&method.name));
        out.push_str("    @staticmethod\n");
        if py_params.is_empty() {
            emit_documented_stub(
                &mut out,
                "    ",
                &format!("def {}() -> {}", method_name, py_return),
                &doc,
                "",
            );
        } else {
            emit_documented_stub(
                &mut out,
                "    ",
                &format!("def {}({}) -> {}", method_name, py_params, py_return),
                &doc,
                "",
            );
        }
    } else {
        let prop_name = to_snake_case(method.name.strip_prefix("get_").unwrap_or(&method.name));
        out.push_str("    @classmethod\n");
        emit_documented_stub(
            &mut out,
            "    ",
            &format!("def get_{}(cls) -> {}", prop_name, py_return),
            &doc,
            "",
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{ParamDirection, ParamMeta};

    fn event_add() -> MethodMeta {
        MethodMeta {
            name: "add_Changed".into(),
            raw_name: "add_Changed".into(),
            params: vec![ParamMeta {
                name: "handler".into(),
                typ: TypeMeta::Parameterized {
                    namespace: "Windows.Foundation".into(),
                    name: "EventHandler`1".into(),
                    piid: "11111111-1111-1111-1111-111111111111".into(),
                    args: vec![TypeMeta::Object],
                },
                direction: ParamDirection::In,
            }],
            is_event_add: true,
            ..Default::default()
        }
    }

    #[test]
    fn paired_event_stubs_preserve_token_api_and_add_helpers() {
        let code = emit_method_stub(
            &event_add(),
            &PythonProjectionContext::default(),
            4,
            true,
            true,
            false,
        );
        assert!(code.contains("def on_changed("));
        assert!(code.contains("-> 'DynWinRTValue': ..."));
        assert!(code.contains("def subscribe_changed("));
        assert!(code.contains("def once_changed("));
    }

    #[test]
    fn add_only_event_stub_does_not_advertise_unavailable_helpers() {
        let code = emit_method_stub(
            &event_add(),
            &PythonProjectionContext::default(),
            4,
            false,
            true,
            false,
        );
        assert!(code.contains("def on_changed("));
        assert!(!code.contains("subscribe_changed"));
        assert!(!code.contains("once_changed"));
    }

    #[test]
    fn append_ignores_only_nullable_reference_override_mismatches() {
        let append = |typ| MethodMeta {
            name: "Append".into(),
            raw_name: "Append".into(),
            params: vec![ParamMeta {
                name: "value".into(),
                typ,
                direction: ParamDirection::In,
            }],
            ..Default::default()
        };
        let reference_type = TypeMeta::Interface {
            namespace: "Contoso".into(),
            name: "Widget".into(),
            iid: "11111111-1111-1111-1111-111111111111".into(),
        };
        let context =
            PythonProjectionContext::standalone([reference_type.type_identity()]).unwrap();
        let reference = emit_method_stub(&append(reference_type), &context, 4, false, true, true);
        let scalar = emit_method_stub(
            &append(TypeMeta::I32),
            &PythonProjectionContext::default(),
            4,
            false,
            true,
            true,
        );

        assert!(reference.contains("type: ignore[override, unused-ignore]"));
        assert!(!scalar.contains("type: ignore[override"));
    }

    #[test]
    fn ambiguous_enum_struct_fields_use_semantic_reference_aliases() {
        let enumeration = |namespace: &str| TypeMeta::Enum {
            namespace: namespace.into(),
            name: "Mode".into(),
            underlying: Box::new(TypeMeta::I32),
            members: vec![],
            is_flags: false,
            doc: None,
            deprecated: None,
        };
        let left = enumeration("Example.Left");
        let right = enumeration("Example.Right");
        let context =
            PythonProjectionContext::packaged([left.type_identity(), right.type_identity()])
                .unwrap();

        assert_eq!(
            py_struct_field_stub_type(&context, &left),
            "'Example_Left_Mode_enum'"
        );
        assert_eq!(
            py_struct_field_read_type(&context, &right),
            "'Example_Right_Mode_enum'"
        );
    }
}
