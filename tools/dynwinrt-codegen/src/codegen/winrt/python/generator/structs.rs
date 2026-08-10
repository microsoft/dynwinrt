// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! Python struct projection helpers.

use super::*;
use crate::codegen::winrt::python::native_types::{FoundationType, foundation_type};
use crate::codegen::winrt::python::type_helpers::py_optional_type;

// ======================================================================
// Struct helpers: Python dataclass-style + _unpack/_pack functions
// ======================================================================

pub(super) fn generate_struct_helpers(s: &TypeMeta) -> String {
    if let Some(kind) = foundation_type(s) {
        return generate_foundation_struct_helpers(s, kind);
    }

    let (namespace, name, fields) = match s {
        TypeMeta::Struct {
            namespace,
            name,
            fields,
        } => (namespace, name, fields),
        _ => return String::new(),
    };
    let mut out = String::new();
    let snake_name = to_snake_case(name);

    // Python class with typed fields
    out.push_str(&format!("\nclass {}:\n", name));
    if fields.is_empty() {
        out.push_str("    pass\n");
    } else {
        // __init__ with typed fields
        let init_params: Vec<String> = fields
            .iter()
            .map(|f| {
                format!(
                    "{}: {} = {}",
                    to_snake_case(&f.name),
                    py_struct_constructor_field_type(&f.typ),
                    py_default_value(&f.typ)
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
                read_type = py_struct_field_read_type(&f.typ),
                write_type = py_struct_field_type(&f.typ),
            ));
        }
    }
    out.push('\n');

    // unpack function
    out.push_str(&format!(
        "\ndef unpack_{}(v: DynWinRTValue) -> {}:\n",
        snake_name, name
    ));
    out.push_str("    s = v.as_struct()\n");
    let field_args: Vec<String> = fields
        .iter()
        .enumerate()
        .map(|(i, f)| {
            format!(
                "{}={}",
                to_snake_case(&f.name),
                py_struct_field_getter(&f.typ, i)
            )
        })
        .collect();
    out.push_str(&format!("    return {}({})\n", name, field_args.join(", ")));
    // Internal alias
    out.push_str(&format!("_unpack_{0} = unpack_{0}\n", snake_name));

    // Type constant
    let full_name = format!("{}.{}", namespace, name);
    let field_types: Vec<String> = fields.iter().map(|f| py_dynwinrt_type(&f.typ)).collect();
    out.push_str(&format!(
        "{}_TYPE = DynWinRTType.struct_type('{}', [{}])\n",
        name,
        full_name,
        field_types.join(", ")
    ));
    out.push_str(&format!("_{0}_TYPE = {0}_TYPE\n", name));

    // pack function
    out.push_str(&format!(
        "\ndef pack_{}(v: {}) -> DynWinRTStruct:\n",
        snake_name, name
    ));
    out.push_str(&format!("    s = DynWinRTStruct.create({}_TYPE)\n", name));
    for (i, f) in fields.iter().enumerate() {
        out.push_str(&format!(
            "    {}\n",
            py_struct_field_setter(&f.typ, i, &format!("v.{}", to_snake_case(&f.name)))
        ));
    }
    out.push_str("    return s\n");
    out.push_str(&format!("_pack_{0} = pack_{0}\n", snake_name));

    out
}

fn generate_foundation_struct_helpers(s: &TypeMeta, kind: FoundationType) -> String {
    let TypeMeta::Struct {
        namespace,
        name,
        fields,
    } = s
    else {
        unreachable!()
    };
    let snake_name = to_snake_case(name);
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
        "\ndef unpack_{snake_name}(v: DynWinRTValue) -> {native_type}:\n\
         \x20   return {from_ticks}(v.as_struct().get_i64(0))\n\
         _unpack_{snake_name} = unpack_{snake_name}\n\
         {name}_TYPE = DynWinRTType.struct_type('{namespace}.{name}', [{field_types}])\n\
         _{name}_TYPE = {name}_TYPE\n\
         \n\
         def pack_{snake_name}(v: {native_type}) -> DynWinRTStruct:\n\
         \x20   s = DynWinRTStruct.create({name}_TYPE)\n\
         \x20   s.set_i64(0, {to_ticks}(v))\n\
         \x20   return s\n\
         _pack_{snake_name} = pack_{snake_name}\n"
    )
}

/// Python default value for struct field initialization.
pub(super) fn py_default_value(typ: &TypeMeta) -> String {
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
        TypeMeta::Enum {
            namespace, name, ..
        } => format!(
            "_dynwinrt_enum('{}', '{}', 0)",
            python_module_name(namespace, name),
            name
        ),
        TypeMeta::Char16 => "'\\0'".to_string(),
        TypeMeta::F32 | TypeMeta::F64 => "0.0".to_string(),
        TypeMeta::String => "''".to_string(),
        TypeMeta::Guid => "UUID(int=0)".to_string(),
        TypeMeta::Struct { name, .. } if name == "HResult" => "0".to_string(),
        _ => "None".to_string(),
    }
}

fn py_struct_constructor_field_type(typ: &TypeMeta) -> String {
    match typ {
        TypeMeta::Struct { name, .. } if foundation_type(typ).is_none() && name != "HResult" => {
            py_optional_type(py_struct_field_type(typ))
        }
        _ => py_struct_field_type(typ),
    }
}

/// Returns the exported names for struct helpers (for Python index).
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
