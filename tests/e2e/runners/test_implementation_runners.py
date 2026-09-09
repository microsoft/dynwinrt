# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import json
import gc
from pathlib import Path
import sys
import unittest
import weakref

from implementation_py import (
    CASES, PUBLIC_VIEW_GC_CASES, RESULT_PREFIX, TaskInstanceHandlers,
    decode_child_result, gc_referent_ids, import_search_path,
)


class ImplementationRunnerTests(unittest.TestCase):
    def test_success_requires_successful_process_cleanup(self):
        stdout = RESULT_PREFIX + json.dumps({"id": "case", "pass": True, "error": None})
        self.assertTrue(decode_child_result("case", stdout, "", 0)["pass"])
        result = decode_child_result("case", stdout, "native failure", 0xC0000005)
        self.assertFalse(result["pass"])
        self.assertIn("cleanup", result["error"])
        self.assertEqual(result["stderr"], "native failure")

    def test_missing_or_malformed_marker_is_not_a_skip_or_success(self):
        for stdout, exit_code in (
            ("", 0xC0000005),
            ("no marker", 0),
            (RESULT_PREFIX + "invalid", 0),
            (RESULT_PREFIX + '{"id":"wrong","pass":true}', 0),
            (RESULT_PREFIX + '{"id":"case","pass":"true"}', 0),
        ):
            with self.subTest(stdout=stdout, exit_code=exit_code):
                self.assertFalse(decode_child_result("case", stdout, "", exit_code)["pass"])

    def test_complete_matrix_and_nine_method_background_fixture(self):
        self.assertEqual(len(CASES), 19)
        self.assertEqual(next(iter(CASES)), "management_handle")
        for case in ("memory_buffer_event", "array_contracts", "fill_array", "named_outputs"):
            self.assertIn(case, CASES)
        self.assertIn("nullable_reference_results", CASES)
        methods = {
            name for name, value in vars(TaskInstanceHandlers).items()
            if callable(value) and not name.startswith("_")
        }
        self.assertEqual(methods, {
            "get_instance_id", "get_task", "get_progress", "set_progress",
            "get_trigger_details", "add_canceled", "remove_canceled",
            "get_suspended_count", "get_deferral",
        })

    def test_public_helper_gc_cases_cannot_be_hidden_by_cleanup_roots(self):
        self.assertEqual(PUBLIC_VIEW_GC_CASES, {
            "public_view_success_gc", "public_view_failed_cast_gc",
        })
        self.assertTrue(PUBLIC_VIEW_GC_CASES <= CASES.keys())

    def test_gc_referent_snapshot_does_not_retain_the_observed_graph(self):
        class Graph:
            pass

        graph = Graph()
        graph.cycle = graph
        reference = weakref.ref(graph)
        snapshot = gc_referent_ids(graph)
        self.assertTrue(snapshot)
        del graph
        gc.collect()
        self.assertIsNone(reference())
        self.assertTrue(all(isinstance(value, int) for value in snapshot))

    def test_generated_import_root_supports_extended_windows_paths(self):
        root = Path(__file__).resolve().parent
        search = import_search_path(root)
        self.assertTrue(Path(search).is_dir())
        self.assertEqual(import_search_path(Path(search)), search)
        if sys.platform == "win32":
            self.assertTrue(search.startswith("\\\\?\\"))
        else:
            self.assertEqual(search, str(root))


if __name__ == "__main__":
    unittest.main()
