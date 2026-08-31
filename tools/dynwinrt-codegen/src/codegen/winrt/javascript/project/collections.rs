// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

//! WinRT collection convenience projections.

use super::*;
use crate::codegen::winrt::javascript::JavaScriptProjectionContext;

// ======================================================================
// Collection helpers
// ======================================================================

pub(super) fn should_skip_raw_collection_method(iface: &InterfaceMeta, method_name: &str) -> bool {
    match iface.generic_piid.as_deref() {
        Some(PIID_IVECTOR | PIID_IVECTOR_VIEW) => match method_name {
            "IndexOf" => true,
            "GetMany" => iface
                .generic_args
                .first()
                .is_some_and(|elem| ts_fill_array_create("count", elem).is_some()),
            "ReplaceAll" => {
                iface.generic_piid.as_deref() == Some(PIID_IVECTOR)
                    && iface
                        .generic_args
                        .first()
                        .is_some_and(|elem| ts_array_from_items("items", elem).is_some())
            }
            _ => false,
        },
        Some(PIID_IMAP_VIEW) => method_name == "Split",
        _ => false,
    }
}

/// Create a fill-array expression for getMany: allocates a DynWinRtArray of
/// `count_var` elements, pre-filled with type-appropriate defaults.
/// Returns `None` for element types that have no typed batch constructor.
fn ts_fill_array_create(count_var: &str, elem: &TypeMeta) -> Option<String> {
    let (method, fill) = match elem {
        TypeMeta::I8 => ("fromI8Values", "0"),
        TypeMeta::U8 => ("fromU8Values", "0"),
        TypeMeta::I16 => ("fromI16Values", "0"),
        TypeMeta::U16 | TypeMeta::Char16 => ("fromU16Values", "0"),
        TypeMeta::I32 | TypeMeta::Enum { .. } => ("fromI32Values", "0"),
        TypeMeta::U32 => ("fromU32Values", "0"),
        TypeMeta::I64 => ("fromI64Values", "0"),
        TypeMeta::U64 => ("fromU64Values", "0"),
        TypeMeta::F32 => ("fromF32Values", "0"),
        TypeMeta::F64 => ("fromF64Values", "0"),
        TypeMeta::String => ("fromStringValues", "''"),
        _ => return None,
    };
    Some(format!(
        "DynWinRtArray.{}(new Array({}).fill({}))",
        method, count_var, fill
    ))
}

/// Create a DynWinRtArray from a JS array variable for replaceAll.
/// Returns `None` for element types that have no typed batch constructor.
fn ts_array_from_items(items_var: &str, elem: &TypeMeta) -> Option<String> {
    let method = match elem {
        TypeMeta::I8 => "fromI8Values",
        TypeMeta::U8 => "fromU8Values",
        TypeMeta::I16 => "fromI16Values",
        TypeMeta::U16 | TypeMeta::Char16 => "fromU16Values",
        TypeMeta::I32 | TypeMeta::Enum { .. } => "fromI32Values",
        TypeMeta::U32 => "fromU32Values",
        TypeMeta::I64 => "fromI64Values",
        TypeMeta::U64 => "fromU64Values",
        TypeMeta::F32 => "fromF32Values",
        TypeMeta::F64 => "fromF64Values",
        TypeMeta::String => "fromStringValues",
        _ => return None,
    };
    Some(format!("DynWinRtArray.{}({})", method, items_var))
}

pub(super) fn project_collection_helpers(
    context: &JavaScriptProjectionContext,
    iface: &InterfaceMeta,
    known_types: &HashSet<String>,
    members: &mut Vec<ProjectedMember>,
    imports: &mut Vec<ProjectedImport>,
    object_expr: &str,
) {
    let Some(piid) = iface.generic_piid.as_deref() else {
        return;
    };

    // If the generic arg is a known parameterized type, it needs to be imported
    // for the IterableIterator<T> / T[] type annotations in DTS
    if !iface.generic_args.is_empty() {
        for arg in &iface.generic_args {
            let type_name = ts_param_type_safe(context, arg, known_types);
            // Primitive types (string, number, boolean, DynWinRtValue, any) don't need import
            if ![
                "string",
                "number",
                "boolean",
                "DynWinRtValue",
                "DynWinRtArray",
                "any",
                "void",
            ]
            .contains(&type_name.as_str())
                && known_types.contains(&type_name)
            {
                let already_imported = imports.iter().any(|i| i.symbols.contains(&type_name));
                if !already_imported {
                    imports.push(ProjectedImport {
                        symbols: vec![type_name],
                        from: format!("./{}.js", ts_param_type_safe(context, arg, known_types)),
                        runtime_only: false,
                        dts_only: false,
                        is_runtime_package: false,
                    });
                }
            }
        }
    }

    match piid {
        PIID_IVECTOR | PIID_IVECTOR_VIEW if iface.generic_args.len() == 1 => {
            let elem_ts = ts_param_type_safe(context, &iface.generic_args[0], known_types);
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::CollectionLength,
                doc: Some(
                    "Alias for {@link size}; matches Array.length / TypedArray.length.".into(),
                ),
            }));
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::CollectionAt { element_type: elem_ts.clone() },
                doc: Some("Element at `index`. Negative indices count from the end (Array.prototype.at semantics).".into()),
            }));
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::CollectionToArray {
                    element_type: elem_ts.clone(),
                },
                doc: Some("Materialize as a plain JS array.".into()),
            }));
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::Iterator {
                    element_type: elem_ts.clone(),
                    body_lines: vec![
                        "const n = this.size;".into(),
                        "for (let i = 0; i < n; i++) yield this.getAt(i);".into(),
                    ],
                },
                doc: None,
            }));

            // indexOf: delegate to WinRT's native IndexOf (returns [u32 index, bool found] via invokeAll)
            let index_of_vtable = iface
                .methods
                .iter()
                .find(|m| m.name == "IndexOf")
                .map(|m| m.vtable_index);
            let iface_var_ref = format!("_{}", iface.name);
            if let Some(idx) = index_of_vtable {
                let wrap_value = wrap_arg(context, "value", &iface.generic_args[0]);
                members.push(ProjectedMember::Method(ProjectedMethod {
                    name: "indexOf".into(),
                    doc: Some(DocInfo {
                        summary: Some("Return the index of `value`, or -1 if not found.".into()),
                        deprecated: None, returns: None, params: vec![],
                    }),
                    params: vec![ProjectedParam {
                        name: "value".into(),
                        ts_type: elem_ts.clone(),
                        optional: false,
                        delegate_wrap: None,
                    }],
                    argument_kinds: vec![],
                    return_type: "number".into(),
                    async_kind: AsyncKind::None,
                    is_static: false,
                    invoke_expr: String::new(),
                    sync_return_expr: Some(format!(
                        "(() => {{ const _r = {iface_var_ref}.method({idx}).invokeAll({object_expr}, [{wrap_value}]); return _r[1].toBool() ? _r[0].toNumber() : -1; }})()"
                    )),
                    async_convert_v: None,
                    is_void: false,
                    array_return_expr: None,
                    delegate_wraps: vec![],
                    progress_convert: None,
                    js_only: false, overload_of: None,
                }));
            }

            // High-level getMany: T[] wrapper over the raw FillArray-based method
            let iface_var = format!("_{}", iface.name);
            let elem = &iface.generic_args[0];
            if let Some(get_many) = iface.methods.iter().find(|m| m.name == "GetMany") {
                if let Some(fill_expr) = ts_fill_array_create("count", elem) {
                    let fill_index =
                        fill_array_output_index(get_many).expect("GetMany FillArray output");
                    let count_index = method_abi_output_count(get_many) - 1;
                    let invoke = format!(
                        "{iface_var}.method({get_many_idx}).invokeAll({object_expr}, \
                         [DynWinRtValue.u32(startIndex), _a.toValue()])",
                        get_many_idx = get_many.vtable_index
                    );
                    let arr_convert = convert_array_return(
                        context,
                        &format!("_r[{fill_index}].asArray()"),
                        elem,
                        known_types,
                        &NO_DEFERRED,
                    );
                    let return_expr = format!(
                        "(() => {{ const _a = {fill_expr}; const _r = {invoke}; \
                         return {arr_convert}.slice(0, _r[{count_index}].toNumber()); }})()"
                    );
                    members.push(ProjectedMember::Method(ProjectedMethod {
                        name: "getMany".into(),
                        doc: Some(DocInfo {
                            summary: Some(
                                "Copy elements starting at `startIndex` into a new array \
                                 of length `count`."
                                    .into(),
                            ),
                            params: vec![],
                            returns: None,
                            deprecated: None,
                        }),
                        params: vec![
                            ProjectedParam {
                                name: "startIndex".into(),
                                ts_type: "number".into(),
                                optional: false,
                                delegate_wrap: None,
                            },
                            ProjectedParam {
                                name: "count".into(),
                                ts_type: "number".into(),
                                optional: false,
                                delegate_wrap: None,
                            },
                        ],
                        argument_kinds: vec![],
                        return_type: format!("{}[]", elem_ts),
                        async_kind: AsyncKind::None,
                        is_static: false,
                        invoke_expr: String::new(),
                        sync_return_expr: Some(return_expr),
                        async_convert_v: None,
                        is_void: false,
                        array_return_expr: None,
                        delegate_wraps: vec![],
                        progress_convert: None,
                        js_only: false,
                        overload_of: None,
                    }));
                }
            }

            // High-level replaceAll (IVector only): accepts T[] instead of DynWinRtArray
            if piid == PIID_IVECTOR {
                if let Some(replace_all_idx) = iface
                    .methods
                    .iter()
                    .find(|m| m.name == "ReplaceAll")
                    .map(|m| m.vtable_index)
                {
                    if let Some(items_expr) = ts_array_from_items("items", elem) {
                        let invoke = format!(
                            "{iface_var}.method({replace_all_idx}).invoke({object_expr}, \
                             [{items_expr}.toValue()])"
                        );
                        members.push(ProjectedMember::Method(ProjectedMethod {
                            name: "replaceAll".into(),
                            doc: Some(DocInfo {
                                summary: Some(
                                    "Replace all elements in the vector with the provided items."
                                        .into(),
                                ),
                                params: vec![],
                                returns: None,
                                deprecated: None,
                            }),
                            params: vec![ProjectedParam {
                                name: "items".into(),
                                ts_type: format!("{}[]", elem_ts),
                                optional: false,
                                delegate_wrap: None,
                            }],
                            argument_kinds: vec![],
                            return_type: "void".into(),
                            async_kind: AsyncKind::None,
                            is_static: false,
                            invoke_expr: invoke,
                            sync_return_expr: None,
                            async_convert_v: None,
                            is_void: true,
                            array_return_expr: None,
                            delegate_wraps: vec![],
                            progress_convert: None,
                            js_only: false,
                            overload_of: None,
                        }));
                    }
                }
            }
        }
        PIID_IITERATOR if iface.generic_args.len() == 1 => {
            let elem_ts = ts_param_type_safe(context, &iface.generic_args[0], known_types);
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::IteratorNext {
                    element_type: elem_ts.clone(),
                },
                doc: Some("JS iterator protocol: returns the current element and advances.".into()),
            }));
            // IIterator is already the iterator — [Symbol.iterator]() returns this
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::Iterator {
                    element_type: elem_ts,
                    body_lines: vec!["return this;".into()],
                },
                doc: None,
            }));
        }
        PIID_IITERABLE if iface.generic_args.len() == 1 => {
            let elem_ts = ts_param_type_safe(context, &iface.generic_args[0], known_types);
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::Iterator {
                    element_type: elem_ts,
                    body_lines: vec![],
                },
                doc: None,
            }));
        }
        PIID_IMAP | PIID_IMAP_VIEW if iface.generic_args.len() == 2 => {
            let key_ts = ts_param_type_safe(context, &iface.generic_args[0], known_types);
            let val_ts = ts_param_type_safe(context, &iface.generic_args[1], known_types);
            let key_ts = if key_ts == "DynWinRtValue" {
                "unknown".to_string()
            } else {
                key_ts
            };
            let val_ts = if val_ts == "DynWinRtValue" {
                "unknown".to_string()
            } else {
                val_ts
            };
            // JS Map-like aliases
            let iface_var = format!("_{}", iface.name);
            // get(key) — alias for lookup
            if let Some(lookup_idx) = iface
                .methods
                .iter()
                .find(|m| m.name == "Lookup")
                .map(|m| m.vtable_index)
            {
                let key_wrap = wrap_arg(context, "key", &iface.generic_args[0]);
                let return_convert = convert_return(
                    context,
                    &format!(
                        "{iface_var}.method({lookup_idx}).invoke({object_expr}, [{key_wrap}])"
                    ),
                    Some(&iface.generic_args[1]),
                    false,
                    known_types,
                    &NO_DEFERRED,
                );
                members.push(ProjectedMember::Method(ProjectedMethod {
                    name: "get".into(),
                    doc: Some(DocInfo {
                        summary: Some(format!("Get the value for `key`. Alias for `lookup()`.")),
                        deprecated: None,
                        returns: None,
                        params: vec![],
                    }),
                    params: vec![ProjectedParam {
                        name: "key".into(),
                        ts_type: key_ts.clone(),
                        optional: false,
                        delegate_wrap: None,
                    }],
                    argument_kinds: vec![],
                    return_type: format!("{} | undefined", val_ts),
                    async_kind: AsyncKind::None,
                    is_static: false,
                    invoke_expr: String::new(),
                    sync_return_expr: Some(format!(
                        "(() => {{ try {{ return {}; }} catch {{ return undefined; }} }})()",
                        return_convert
                    )),
                    async_convert_v: None,
                    is_void: false,
                    array_return_expr: None,
                    delegate_wraps: vec![],
                    progress_convert: None,
                    js_only: false,
                    overload_of: None,
                }));
            }
            // has(key) — alias for hasKey
            if let Some(has_idx) = iface
                .methods
                .iter()
                .find(|m| m.name == "HasKey")
                .map(|m| m.vtable_index)
            {
                let key_wrap = wrap_arg(context, "key", &iface.generic_args[0]);
                members.push(ProjectedMember::Method(ProjectedMethod {
                    name: "has".into(),
                    doc: Some(DocInfo {
                        summary: Some(
                            "Check if the map contains `key`. Alias for `hasKey()`.".into(),
                        ),
                        deprecated: None,
                        returns: None,
                        params: vec![],
                    }),
                    params: vec![ProjectedParam {
                        name: "key".into(),
                        ts_type: key_ts.clone(),
                        optional: false,
                        delegate_wrap: None,
                    }],
                    argument_kinds: vec![],
                    return_type: "boolean".into(),
                    async_kind: AsyncKind::None,
                    is_static: false,
                    invoke_expr: format!(
                        "{iface_var}.method({has_idx}).invoke({object_expr}, [{key_wrap}])"
                    ),
                    sync_return_expr: Some(format!(
                        "{iface_var}.method({has_idx}).invoke({object_expr}, [{key_wrap}]).toBool()"
                    )),
                    async_convert_v: None,
                    is_void: false,
                    array_return_expr: None,
                    delegate_wraps: vec![],
                    progress_convert: None,
                    js_only: false,
                    overload_of: None,
                }));
            }
            // set(key, value) — alias for insert (IMap only)
            if piid == PIID_IMAP {
                if let Some(insert_idx) = iface
                    .methods
                    .iter()
                    .find(|m| m.name == "Insert")
                    .map(|m| m.vtable_index)
                {
                    let key_wrap = wrap_arg(context, "key", &iface.generic_args[0]);
                    let val_wrap = wrap_arg(context, "value", &iface.generic_args[1]);
                    members.push(ProjectedMember::Method(ProjectedMethod {
                        name: "set".into(),
                        doc: Some(DocInfo {
                            summary: Some("Set a key-value pair. Alias for `insert()`.".into()),
                            deprecated: None, returns: None, params: vec![],
                        }),
                        params: vec![
                            ProjectedParam { name: "key".into(), ts_type: key_ts.clone(), optional: false, delegate_wrap: None },
                            ProjectedParam { name: "value".into(), ts_type: val_ts.clone(), optional: false, delegate_wrap: None },
                        ],
                        argument_kinds: vec![],
                        return_type: "void".into(),
                        async_kind: AsyncKind::None, is_static: false,
                        invoke_expr: format!("{iface_var}.method({insert_idx}).invoke({object_expr}, [{key_wrap}, {val_wrap}])"),
                        sync_return_expr: None,
                        async_convert_v: None, is_void: true, array_return_expr: None,
                        delegate_wraps: vec![], progress_convert: None, js_only: false, overload_of: None,
                    }));
                }
                // delete(key) — alias for remove
                if let Some(remove_idx) = iface
                    .methods
                    .iter()
                    .find(|m| m.name == "Remove")
                    .map(|m| m.vtable_index)
                {
                    let key_wrap = wrap_arg(context, "key", &iface.generic_args[0]);
                    members.push(ProjectedMember::Method(ProjectedMethod {
                        name: "delete".into(),
                        doc: Some(DocInfo {
                            summary: Some("Remove entry by key. Alias for `remove()`.".into()),
                            deprecated: None,
                            returns: None,
                            params: vec![],
                        }),
                        params: vec![ProjectedParam {
                            name: "key".into(),
                            ts_type: key_ts.clone(),
                            optional: false,
                            delegate_wrap: None,
                        }],
                        argument_kinds: vec![],
                        return_type: "void".into(),
                        async_kind: AsyncKind::None,
                        is_static: false,
                        invoke_expr: format!(
                            "{iface_var}.method({remove_idx}).invoke({object_expr}, [{key_wrap}])"
                        ),
                        sync_return_expr: None,
                        async_convert_v: None,
                        is_void: true,
                        array_return_expr: None,
                        delegate_wraps: vec![],
                        progress_convert: None,
                        js_only: false,
                        overload_of: None,
                    }));
                }
            }
            // forEach — iterate over entries
            members.push(ProjectedMember::Symbol(ProjectedSymbol {
                kind: SymbolKind::CollectionLength,
                doc: Some("Number of entries. Alias for `size`.".into()),
            }));
        }
        _ => {}
    }
}

pub(super) fn project_collection_create(
    context: &JavaScriptProjectionContext,
    iface: &InterfaceMeta,
    known_types: &HashSet<String>,
    members: &mut Vec<ProjectedMember>,
    imports: &mut Vec<ProjectedImport>,
) {
    let Some(ref piid) = iface.generic_piid else {
        return;
    };
    if piid == PIID_IVECTOR && iface.generic_args.len() == 1 {
        let elem_type = ts_dynwinrt_type(context, &iface.generic_args[0]);
        let elem_ts = ts_param_type_safe(context, &iface.generic_args[0], known_types);
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "create".into(),
            doc: Some(DocInfo {
                summary: Some("Create a new IVector from an array of items.".into()),
                deprecated: None,
                returns: None,
                params: vec![],
            }),
            params: vec![ProjectedParam {
                name: "items".into(),
                ts_type: format!("{}[]", elem_ts),
                optional: false,
                delegate_wrap: None,
            }],
            argument_kinds: vec![],
            return_type: iface.name.clone(),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "new {}(DynWinRtValue.createVector(items.map(i => _unwrap(i)), {}))",
                iface.name, elem_type
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false,
            overload_of: None,
        }));
    } else if piid == PIID_IOBSERVABLE_VECTOR && iface.generic_args.len() == 1 {
        let elem_type = ts_dynwinrt_type(context, &iface.generic_args[0]);
        let elem_ts = ts_param_type_safe(context, &iface.generic_args[0], known_types);
        let vector_name = context.projected_parameterized_name(
            crate::meta::WINDOWS_FOUNDATION_COLLECTIONS_NAMESPACE,
            "IVector",
            PIID_IVECTOR,
            &iface.generic_args,
        );
        imports.push(ProjectedImport {
            symbols: vec![vector_name.clone()],
            from: format!("./{}.js", vector_name),
            runtime_only: false,
            dts_only: false,
            is_runtime_package: false,
        });
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "asVector".into(),
            doc: Some(DocInfo {
                summary: Some(
                    "Cast this observable collection to its mutable vector interface.".into(),
                ),
                deprecated: None,
                returns: None,
                params: vec![],
            }),
            params: vec![],
            argument_kinds: vec![],
            return_type: vector_name.clone(),
            async_kind: AsyncKind::None,
            is_static: false,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "new {vector}(this._obj)",
                vector = ref_marker(&vector_name),
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false,
            overload_of: None,
        }));
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "create".into(),
            doc: Some(DocInfo {
                summary: Some(
                    "Create an observable mutable vector from an array of items."
                        .into(),
                ),
                deprecated: None,
                returns: None,
                params: vec![],
            }),
            params: vec![ProjectedParam {
                name: "items".into(),
                ts_type: format!("{}[]", elem_ts),
                optional: false,
                delegate_wrap: None,
            }],
            argument_kinds: vec![],
            return_type: format!(
                "{} & {}",
                iface.name, vector_name,
            ),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "(() => {{ const value = DynWinRtValue.createVector(items.map(i => _unwrap(i)), {elem_type}); const observable = new {observable}(value); const vector = new {vector}(value); Object.defineProperties(vector, {{ asVector: {{ value: observable.asVector.bind(observable) }}, onVectorChanged: {{ value: observable.onVectorChanged.bind(observable) }}, onceVectorChanged: {{ value: observable.onceVectorChanged.bind(observable) }}, offVectorChanged: {{ value: observable.offVectorChanged.bind(observable) }} }}); return vector; }})()",
                observable = iface.name,
                vector = ref_marker(&vector_name),
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false,
            overload_of: None,
        }));
    } else if piid == PIID_IMAP && iface.generic_args.len() == 2 {
        let key_type = ts_dynwinrt_type(context, &iface.generic_args[0]);
        let val_type = ts_dynwinrt_type(context, &iface.generic_args[1]);
        let key_ts = ts_param_type_safe(context, &iface.generic_args[0], known_types);
        let key_ts = if key_ts == "DynWinRtValue" {
            "unknown".to_string()
        } else {
            key_ts
        };
        let val_ts = ts_param_type_safe(context, &iface.generic_args[1], known_types);
        let val_ts = if val_ts == "DynWinRtValue" {
            "unknown".to_string()
        } else {
            val_ts
        };
        members.push(ProjectedMember::Method(ProjectedMethod {
            name: "create".into(),
            doc: Some(DocInfo {
                summary: Some("Create a new IMap from parallel arrays of keys and values.".into()),
                deprecated: None, returns: None, params: vec![],
            }),
            params: vec![
                ProjectedParam { name: "keys".into(), ts_type: format!("{}[]", key_ts), optional: false, delegate_wrap: None },
                ProjectedParam { name: "values".into(), ts_type: format!("{}[]", val_ts), optional: false, delegate_wrap: None },
            ],
            argument_kinds: vec![],
            return_type: iface.name.clone(),
            async_kind: AsyncKind::None,
            is_static: true,
            invoke_expr: String::new(),
            sync_return_expr: Some(format!(
                "new {}(DynWinRtValue.createMap(keys.map(k => _unwrap(k)), values.map(v => _unwrap(v)), {}, {}))",
                iface.name, key_type, val_type
            )),
            async_convert_v: None,
            is_void: false,
            array_return_expr: None,
            delegate_wraps: vec![],
            progress_convert: None,
            js_only: false, overload_of: None,
        }));
    }
}
