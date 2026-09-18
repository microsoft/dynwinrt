// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::BTreeMap;

use dynwinrt_codegen::{codegen::win32, win32_metadata};
use serde::Serialize;

#[derive(Serialize)]
struct Census {
    metadata: String,
    eligible_functions: usize,
    complete_functions: usize,
    omitted_functions: usize,
    coverage_percent: f64,
    omission_reasons: BTreeMap<String, usize>,
}

pub(super) fn run(winmd: &str, json: bool) -> Result<(), String> {
    let functions = win32_metadata::parse_all_functions(winmd)
        .ok_or_else(|| format!("Failed to load flat Win32 metadata from {winmd}"))?;
    let eligible = functions.len();
    let mut containers = BTreeMap::new();
    for function in functions {
        containers
            .entry((function.namespace.clone(), function.container.clone()))
            .or_insert_with(Vec::new)
            .push(function);
    }
    let mut complete = 0;
    let mut reasons = BTreeMap::new();
    for ((namespace, class_name), functions) in containers {
        let projection = win32::project_apis(&win32_metadata::RawApis {
            namespace,
            class_name,
            functions,
        });
        complete += projection.complete_count();
        for omission in projection.omitted {
            *reasons
                .entry(reason_code(&omission.reason).to_string())
                .or_insert(0) += 1;
        }
    }
    let result = Census {
        metadata: winmd.into(),
        eligible_functions: eligible,
        complete_functions: complete,
        omitted_functions: eligible
            .checked_sub(complete)
            .ok_or("Flat Win32 projection counted more exports than metadata contains")?,
        coverage_percent: if eligible == 0 {
            0.0
        } else {
            complete as f64 * 100.0 / eligible as f64
        },
        omission_reasons: reasons,
    };
    if json {
        println!(
            "{}",
            serde_json::to_string(&result)
                .map_err(|error| format!("Failed to serialize Win32 census: {error}"))?
        );
    } else {
        println!(
            "Flat Win32 complete functions: {}/{} ({:.6}%)",
            result.complete_functions, result.eligible_functions, result.coverage_percent
        );
        for (reason, count) in result.omission_reasons {
            println!("  {count:>5}  {reason}");
        }
    }
    Ok(())
}

fn reason_code(reason: &str) -> &'static str {
    if reason.contains("calling convention") {
        "calling-convention"
    } else if reason.contains("variadic") {
        "variadic"
    } else if reason.contains("both x64 and ARM64") {
        "architecture"
    } else if reason.contains("System32 DLL") {
        "module-policy"
    } else if reason.contains("callback thunk") {
        "callback"
    } else if reason.contains("cleanup") {
        "cleanup"
    } else if reason.contains("pointer return lifetime") {
        "return-ownership"
    } else if reason.contains("native buffer") || reason.contains("count parameter") {
        "buffer-contract"
    } else if reason.contains("writable pointer") {
        "writable-pointer"
    } else if reason.contains("NativeStruct") {
        "native-layout"
    } else if reason.contains("pointer depth") || reason.contains("void is not") {
        "pointer-contract"
    } else if reason.contains("enum underlying") {
        "enum-abi"
    } else if reason.contains("unknown") || reason.contains("Unknown") {
        "unknown-native-type"
    } else {
        "other"
    }
}
