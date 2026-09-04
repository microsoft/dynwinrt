// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! JavaScript struct projection.

use super::*;
use crate::codegen::winrt::javascript::JavaScriptProjectionContext;

// ======================================================================
// Struct projection
// ======================================================================

pub(super) fn project_struct_helpers(
    context: &JavaScriptProjectionContext,
    used_structs: &[TypeMeta],
) -> Vec<ProjectedStruct> {
    used_structs
        .iter()
        .filter_map(|s| {
            let (namespace, name, fields) = match s {
                TypeMeta::Struct {
                    namespace,
                    name,
                    fields,
                } => (namespace, name, fields),
                _ => return None,
            };
            let full_name = format!("{}.{}", namespace, name);
            let field_types: Vec<String> = fields
                .iter()
                .map(|f| ts_dynwinrt_type(context, &f.typ))
                .collect();
            let type_expr = format!(
                "DynWinRtType.structType('{}', [{}])",
                full_name,
                field_types.join(", ")
            );

            let ts_fields: Vec<(String, String)> = fields
                .iter()
                .map(|f| {
                    (
                        to_camel_case(&f.name),
                        ts_struct_field_type(context, &f.typ),
                    )
                })
                .collect();

            let unpack_body = {
                let field_exprs: Vec<String> = fields
                    .iter()
                    .enumerate()
                    .map(|(i, f)| {
                        format!(
                            "{}: {}",
                            to_camel_case(&f.name),
                            struct_field_getter(context, &f.typ, i)
                        )
                    })
                    .collect();
                vec![
                    "const s = v.asStruct();".into(),
                    format!("return {{ {} }};", field_exprs.join(", ")),
                ]
            };

            let pack_body = {
                let mut lines = vec![format!("const s = DynWinRtStruct.create({}_Type);", name)];
                for (i, f) in fields.iter().enumerate() {
                    lines.push(format!(
                        "{};",
                        struct_field_setter(
                            context,
                            &f.typ,
                            i,
                            &format!("v.{}", to_camel_case(&f.name)),
                        )
                    ));
                }
                lines.push("return s;".into());
                lines
            };

            Some(ProjectedStruct {
                name: name.clone(),
                fields: ts_fields,
                unpack_body,
                pack_body,
                type_expr,
                namespace: namespace.clone(),
            })
        })
        .collect()
}
