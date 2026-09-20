// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python struct projection helpers.

use super::imports::{emit_type_checking_imports, format_py_type_import};
use super::*;
use crate::codegen::winrt::python::naming::{PythonSymbol, STRUCT_SYMBOLS};
use crate::codegen::winrt::python::native_types::{FoundationType, foundation_type};
use crate::codegen::winrt::python::type_helpers::py_optional_type;
use crate::types::FieldMeta;

pub(super) use super::super::naming::py_struct_export_names;

// ======================================================================
// Struct helpers: Python dataclass-style + _unpack/_pack functions
// ======================================================================

pub(super) fn generate_struct_imports(
    context: &PythonProjectionContext,
    structs: &[TypeMeta],
) -> String {
    let mut imports = structs
        .iter()
        .filter_map(|typ| {
            let TypeMeta::Struct { .. } = typ else {
                return None;
            };
            let names = STRUCT_SYMBOLS
                .into_iter()
                .filter(|role| *role != PythonSymbol::Type || foundation_type(typ).is_none())
                .map(|role| context.symbol_import(&typ.type_identity(), role))
                .collect::<Vec<_>>();
            Some(format!(
                "from .{} import {}  # noqa: F401\n",
                context.implementation_module_for_type(typ),
                names.join(", ")
            ))
        })
        .collect::<Vec<_>>();
    imports.sort();
    imports.dedup();
    imports.concat()
}

pub fn generate_struct(context: &PythonProjectionContext, s: &TypeMeta) -> Option<String> {
    let TypeMeta::Struct { name, .. } = s else {
        return None;
    };
    if name == "HResult" {
        return None;
    }
    let dependencies = collect_used_structs_from_struct(s);
    let mut module_structs = dependencies.clone();
    module_structs.push(s.clone());
    let context = context.for_struct_module(s, &module_structs);
    let context = context.as_ref();

    let mut out = String::new();
    out.push_str(HEADER);
    out.push_str(FUTURE_ANNOTATIONS);
    out.push_str(&import_line(context));

    out.push_str(&generate_struct_imports(context, &dependencies));
    if has_ireference_struct_field(std::slice::from_ref(s)) {
        out.push_str(IREFERENCE_HELPER);
    }
    out.push('\n');

    let mut type_checking_imports = collect_struct_field_type_imports(s)
        .into_iter()
        .map(|type_ref| {
            format_py_type_import(context, &type_ref.namespace, &type_ref.name, type_ref.kind)
        })
        .collect::<Vec<_>>();
    type_checking_imports.extend(
        collect_used_generic_identities_from_type(s)
            .into_iter()
            .map(|identity| {
                let identity = context.normalize_identity(&identity);
                let name = context.projected_name(&identity);
                let reference_name = context.reference_name(&identity);
                let import = if name == reference_name {
                    name
                } else {
                    format!("{name} as {reference_name}")
                };
                format!(
                    "from .{} import {import}  # noqa: F401\n",
                    context.implementation_module(&identity),
                )
            }),
    );
    emit_type_checking_imports(&mut out, type_checking_imports);
    out.push_str(&generate_struct_helpers(context, s));
    Some(out)
}

pub(super) fn generate_struct_helpers(context: &PythonProjectionContext, s: &TypeMeta) -> String {
    if let Some(kind) = foundation_type(s) {
        return generate_foundation_struct_helpers(context, s, kind);
    }

    let (namespace, metadata_name, fields) = match s {
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => (namespace, name, fields),
        _ => return String::new(),
    };
    let mut out = String::new();
    let name = context.struct_symbol(s, PythonSymbol::Type);
    let pack = context.struct_symbol(s, PythonSymbol::Pack);
    let unpack = context.struct_symbol(s, PythonSymbol::Unpack);
    let private_pack = context.struct_symbol(s, PythonSymbol::PrivatePack);
    let private_unpack = context.struct_symbol(s, PythonSymbol::PrivateUnpack);
    let type_constant = context.struct_symbol(s, PythonSymbol::TypeConstant);
    let private_type_constant = context.struct_symbol(s, PythonSymbol::PrivateTypeConstant);
    let field_names = py_struct_field_names(fields);
    let slot_names = fields.iter().map(py_struct_slot_name).collect::<Vec<_>>();

    // Python class with typed fields
    out.push_str(&format!("\nclass {}:\n", name));
    out.push_str(&format!(
        "    __slots__ = {}\n",
        py_string_tuple_literal(&slot_names)
    ));
    out.push('\n');
    if fields.is_empty() {
        out.push_str("    def __init__(self):\n");
        out.push_str("        pass\n");
    } else {
        // __init__ with typed fields
        let init_params: Vec<String> = fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {} = {}",
                    to_snake_case(&f.name),
                    py_struct_constructor_field_type(context, &f.typ),
                    py_default_value(context, &f.typ)
                )
            })
            .collect();
        out.push_str(&format!(
            "    def __init__(self, {}):\n",
            init_params.join(", ")
        ));
        for f in fields {
            let snake = to_snake_case(&f.name);
            if let TypeMeta::Struct { name, .. } = &f.typ
                && foundation_type(&f.typ).is_none()
                && name != "HResult"
            {
                let name = context.reference_name_for_type(&f.typ);
                out.push_str(&format!(
                    "        self.{snake} = {name}() if {snake} is None else {snake}\n"
                ));
            } else {
                out.push_str(&format!("        self.{snake} = {snake}\n"));
            }
        }
        for f in fields {
            if ireference_inner_type(&f.typ).is_none() {
                continue;
            }
            let snake = to_snake_case(&f.name);
            out.push_str(&format!(
                "\n    @_property\n\
                 \x20   def {snake}(self) -> {read_type}:\n\
                 \x20       return self._{snake}\n\
                 \n\
                 \x20   @{snake}.setter\n\
                 \x20   def {snake}(self, value: {write_type}) -> None:\n\
                 \x20       self._{snake} = _dynwinrt_unbox_reference(value)\n",
                read_type = py_struct_field_read_type(context, &f.typ),
                write_type = py_struct_field_type(context, &f.typ),
            ));
        }
    }
    out.push('\n');
    out.push_str("    def __eq__(self, other: object) -> bool:\n");
    out.push_str("        if type(other) is not type(self):\n");
    out.push_str("            return NotImplemented\n");
    if field_names.is_empty() {
        out.push_str("        return True\n");
    } else {
        out.push_str(&format!(
            "        return {} == {}\n",
            py_attribute_tuple_expr("self", &field_names),
            py_attribute_tuple_expr("other", &field_names),
        ));
    }
    out.push('\n');
    out.push_str("    def __repr__(self) -> str:\n");
    out.push_str(&format!(
        "        return {}\n",
        py_struct_repr_expr(&field_names)
    ));
    out.push('\n');

    // unpack function
    out.push_str(&format!("\ndef {unpack}(v: DynWinRTValue) -> {name}:\n",));
    out.push_str("    s = v.as_struct()\n");
    let field_args: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            format!(
                "{}={}",
                to_snake_case(&f.name),
                py_struct_field_getter(context, &f.typ, i)
            )
        })
        .collect();
    out.push_str(&format!("    return {}({})\n", name, field_args.join(", ")));
    // Internal alias
    out.push_str(&format!("{private_unpack} = {unpack}\n"));

    // Type constant
    let full_name = format!("{}.{}", namespace, metadata_name);
    let field_types: Vec<String> = fields.iter().map(|f| py_dynwinrt_type(&f.typ)).collect();
    out.push_str(&format!(
        "{type_constant} = DynWinRTType.struct_type('{}', [{}])\n",
        full_name,
        field_types.join(", ")
    ));
    out.push_str(&format!("{private_type_constant} = {type_constant}\n"));

    // pack function
    out.push_str(&format!("\ndef {pack}(v: {name}) -> DynWinRTStruct:\n",));
    out.push_str(&format!("    s = DynWinRTStruct.create({type_constant})\n"));
    for (i, f) in fields.iter().enumerate() {
        out.push_str(&format!(
            "    {}\n",
            py_struct_field_setter(context, &f.typ, i, &format!("v.{}", to_snake_case(&f.name)))
        ));
    }
    out.push_str("    return s\n");
    out.push_str(&format!("{private_pack} = {pack}\n"));

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

fn py_struct_field_names(fields: &[FieldMeta]) -> Vec<String> {
    fields
        .iter()
        .map(|field| to_snake_case(&field.name))
        .collect()
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

fn py_attribute_tuple_expr(receiver: &str, field_names: &[String]) -> String {
    match field_names {
        [] => "()".to_string(),
        [field] => format!("({receiver}.{field},)"),
        _ => format!(
            "({})",
            field_names
                .iter()
                .map(|field| format!("{receiver}.{field}"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

fn py_struct_repr_expr(field_names: &[String]) -> String {
    if field_names.is_empty() {
        return "f'{type(self).__name__}()'".to_string();
    }

    let fields = field_names
        .iter()
        .map(|field| format!("{field}={{self.{field}!r}}"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("f'{{type(self).__name__}}({fields})'")
}

fn generate_foundation_struct_helpers(
    context: &PythonProjectionContext,
    s: &TypeMeta,
    kind: FoundationType,
) -> String {
    let TypeMeta::Struct {
        namespace,
        name,
        fields,
    } = s
    else {
        unreachable!()
    };
    let pack = context.struct_symbol(s, PythonSymbol::Pack);
    let unpack = context.struct_symbol(s, PythonSymbol::Unpack);
    let private_pack = context.struct_symbol(s, PythonSymbol::PrivatePack);
    let private_unpack = context.struct_symbol(s, PythonSymbol::PrivateUnpack);
    let type_constant = context.struct_symbol(s, PythonSymbol::TypeConstant);
    let private_type_constant = context.struct_symbol(s, PythonSymbol::PrivateTypeConstant);
    let native_type = match kind {
        FoundationType::DateTime => "datetime",
        FoundationType::TimeSpan => "timedelta",
    };
    let from_ticks = match kind {
        FoundationType::DateTime => "_dynwinrt_ticks_to_datetime",
        FoundationType::TimeSpan => "_dynwinrt_ticks_to_timedelta",
    };
    let to_ticks = match kind {
        FoundationType::DateTime => "_dynwinrt_datetime_to_ticks",
        FoundationType::TimeSpan => "_dynwinrt_timedelta_to_ticks",
    };
    let field_types = fields
        .iter()
        .map(|field| py_dynwinrt_type(&field.typ))
        .collect::<Vec<_>>()
        .join(", ");
    format!(
        "\ndef {unpack}(v: DynWinRTValue) -> {native_type}:\n\
         \x20   return {from_ticks}(v.as_struct().get_i64(0))\n\
         {private_unpack} = {unpack}\n\
         {type_constant} = DynWinRTType.struct_type('{namespace}.{name}', [{field_types}])\n\
         {private_type_constant} = {type_constant}\n\
         \n\
         def {pack}(v: {native_type}) -> DynWinRTStruct:\n\
         \x20   s = DynWinRTStruct.create({type_constant})\n\
         \x20   s.set_i64(0, {to_ticks}(v))\n\
         \x20   return s\n\
         {private_pack} = {pack}\n"
    )
}

/// Python default value for struct field initialization.
pub(super) fn py_default_value(context: &PythonProjectionContext, typ: &TypeMeta) -> String {
    match foundation_type(typ) {
        Some(FoundationType::DateTime) => return "_dynwinrt_ticks_to_datetime(0)".to_string(),
        Some(FoundationType::TimeSpan) => return "timedelta(0)".to_string(),
        None => {}
    }

    match typ {
        TypeMeta::Bool => "False".to_string(),
        TypeMeta::I8
        | TypeMeta::U8
        | TypeMeta::I16
        | TypeMeta::U16
        | TypeMeta::I32
        | TypeMeta::U32
        | TypeMeta::I64
        | TypeMeta::U64 => "0".to_string(),
        TypeMeta::Enum { .. } => format!(
            "_dynwinrt_enum('{}', '{}', 0)",
            context.implementation_module_for_type(typ),
            context.projected_name_for_type(typ)
        ),
        TypeMeta::Char16 => "'\\0'".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "0.0".to_string(),
        TypeMeta::String => "''".to_string(),
        TypeMeta::Guid => "UUID(int=0)".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "0".to_string(),
        _ => "None".to_string(),
    }
}

fn py_struct_constructor_field_type(context: &PythonProjectionContext, typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Struct { name, .. } if foundation_type(typ).is_none() && name != "HResult" => {
            py_optional_type(py_struct_field_type(context, typ))
        }
        _ => py_struct_field_type(context, typ),
    }
}
