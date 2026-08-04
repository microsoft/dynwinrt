// Copyright (c) Microsoft Corporation.
// Licensed under the MIT License.

use std::collections::HashSet;

use dynwinrt_codegen::codegen::python;
use dynwinrt_codegen::codegen::python_stub;
use dynwinrt_codegen::meta::ClassMeta;

#[test]
fn dispatcher_queue_emits_awaitable_scheduling_helpers() {
    let class = ClassMeta {
        name: "DispatcherQueue".into(),
        namespace: "Microsoft.UI.Dispatching".into(),
        full_name: "Microsoft.UI.Dispatching.DispatcherQueue".into(),
        ..Default::default()
    };
    let known_types = HashSet::new();
    let delegate_types = HashSet::new();
    let shared_iids = HashSet::new();

    let runtime = python::generate_class(&class, &known_types, &delegate_types, &shared_iids);
    assert!(runtime.contains("import asyncio"));
    assert!(runtime.contains("async def enqueue_async(self, callback, *args, **kwargs):"));
    assert!(runtime.contains(
        "async def enqueue_with_priority_async(self, priority, callback, *args, **kwargs):"
    ));
    assert!(runtime.contains("loop.call_soon_threadsafe(complete, result, error)"));
    assert!(runtime.contains("except RuntimeError:"));
    assert!(runtime.contains("DispatcherQueue rejected the callback."));

    let stub =
        python_stub::generate_class_stub(&class, &known_types, &delegate_types, &shared_iids);
    assert!(stub.contains("_DispatchResultT = TypeVar('_DispatchResultT')"));
    assert!(stub.contains("async def enqueue_async("));
    assert!(stub.contains("Callable[..., _DispatchResultT]"));
    assert!(stub.contains("async def enqueue_with_priority_async("));
}
