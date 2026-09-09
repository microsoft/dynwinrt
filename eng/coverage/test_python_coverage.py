# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import contextlib
import io
import json
import os
from pathlib import Path
import shutil
import unittest
import uuid

from coverage import CoverageData
from coverage.exceptions import NoSource

from python_coverage import normalize_data, normalize_path, write_reports


HERE = Path(__file__).resolve().parent
STANDARD = Path("tests/e2e/e2e_generated/python_bindings")
IMPLEMENTATIONS = Path("tests/e2e/e2e_generated/implementations/python_bindings")


class PathNormalizationTests(unittest.TestCase):
    def test_extended_drive_paths_become_repo_relative(self):
        root = Path(r"D:\a\dynwinrt\dynwinrt")
        relative = str(IMPLEMENTATIONS / "windows" / "i_closable.py")
        for prefix in (r"D:\a\dynwinrt\dynwinrt", r"\\?\D:\a\dynwinrt\dynwinrt"):
            with self.subTest(prefix=prefix):
                self.assertEqual(normalize_path(prefix + "\\" + relative, root), relative)

    def test_extended_unc_paths_and_case_insensitive_root(self):
        self.assertEqual(
            normalize_path(
                r"\\?\UNC\SERVER\Share\Repo\tests\example.py",
                Path(r"\\server\share\repo"),
            ),
            os.path.normpath("tests/example.py"),
        )

    def test_relative_paths_and_other_checkouts_are_not_conflated(self):
        root = Path(r"D:\a\dynwinrt\dynwinrt")
        self.assertEqual(normalize_path("tests/example.py", root), os.path.normpath("tests/example.py"))
        other = r"D:\a\dynwinrt\dynwinrt-other\tests\example.py"
        self.assertEqual(normalize_path(other, root), os.path.normpath(other))

    def test_alias_merging_preserves_arcs_and_contexts(self):
        root = Path(r"D:\a\dynwinrt\dynwinrt")
        data = CoverageData(no_disk=True)
        self.addCleanup(data.close, force=True)
        data.set_context("normal")
        data.add_arcs({str(root / "example.py"): [(1, 2), (2, 4)]})
        data.set_context("extended")
        data.add_arcs({r"\\?\D:\a\dynwinrt\dynwinrt\example.py": [(1, 3), (3, 4)]})
        normalize_data(data, root)

        self.assertEqual(data.measured_files(), {"example.py"})
        self.assertEqual(set(data.arcs("example.py")), {(1, 2), (2, 4), (1, 3), (3, 4)})
        self.assertEqual(data.measured_contexts(), {"normal", "extended"})
        data.set_query_contexts(["extended"])
        self.assertEqual(set(data.arcs("example.py")), {(1, 3), (3, 4)})


class ReportTests(unittest.TestCase):
    def setUp(self):
        self.root = HERE / "testdata" / (".report-test-" + uuid.uuid4().hex)
        self.output = self.root / "reports"
        self.output.mkdir(parents=True)

    def tearDown(self):
        shutil.rmtree("\\\\?\\" + str(self.root) if os.name == "nt" else self.root)

    def source(self, relative: Path, text: str = "def value():\n    return 3\n\nvalue()\n") -> Path:
        path = self.root / relative
        io_path = Path("\\\\?\\" + str(path)) if os.name == "nt" else path
        io_path.parent.mkdir(parents=True, exist_ok=True)
        io_path.write_text(text, encoding="utf-8")
        return path

    def record(self, name: str, lines: dict[str, list[int]]) -> None:
        data = CoverageData(basename=str(self.output / (".coverage." + name)))
        data.set_context(name)
        data.add_lines(lines)
        data.write()

    def report(self, **kwargs):
        with contextlib.redirect_stdout(io.StringIO()):
            write_reports(self.root, self.output, HERE / "python-coveragerc", **kwargs)

    def read_report(self, name: str = "coverage.json") -> dict:
        return json.loads((self.output / name).read_text(encoding="utf-8"))

    def test_aggregate_keeps_both_generated_trees_runtime_and_transitive_interfaces(self):
        standard = self.source(STANDARD / "windows__i_closable.py")
        implementation = self.source(IMPLEMENTATIONS / "windows__i_closable.py")
        transitive = self.source(STANDARD / "windows" / "transitive.py", "def unused():\n    return 7\n")
        runtime = self.source(Path("bindings/py/src/implementation.py"))
        runner = self.source(Path("tests/e2e/runners/runner.py"))
        self.record("standard", {
            str(standard.relative_to(self.root)): [1, 2, 4],
            str(transitive.relative_to(self.root)): [],
            str(runner.relative_to(self.root)): [1, 2, 4],
            str(runtime.relative_to(self.root)): [1, 2, 4],
        })
        self.record("implementations", {
            "\\\\?\\" + str(implementation): [1, 2, 4],
        })

        self.report(require_generated=True)
        aggregate = self.read_report()
        diagnostic = self.read_report("all-data.json")
        self.assertEqual(aggregate["totals"]["covered_lines"], 9)
        self.assertEqual(aggregate["totals"]["num_statements"], 11)
        self.assertEqual(len(aggregate["files"]), 4)
        self.assertEqual(len(diagnostic["files"]), 5)
        for path, entry in aggregate["files"].items():
            self.assertFalse(Path(normalize_path(path, self.root)).is_absolute(), path)
            self.assertEqual(entry["summary"], diagnostic["files"][path]["summary"])
        normalized_files = {
            normalize_path(path, self.root): entry
            for path, entry in aggregate["files"].items()
        }
        missing = normalized_files[str(transitive.relative_to(self.root))]["summary"]
        self.assertEqual(missing["covered_lines"], 0)
        self.assertEqual(missing["num_statements"], 2)
        layers = [
            self.read_report(name + "/coverage.json")
            for name in ("runtime", "generated-winrt", "generated-implementations")
        ]
        for key in ("covered_lines", "num_statements", "num_branches", "covered_branches"):
            self.assertEqual(
                aggregate["totals"].get(key, 0),
                sum(layer["totals"].get(key, 0) for layer in layers),
            )
        for name in ("runtime", "generated-winrt", "generated-implementations"):
            for report in ("html/index.html", "coverage.xml", "lcov.info"):
                self.assertTrue((self.output / name / report).is_file())
        self.assertEqual(len(list(self.output.glob(".coverage.*"))), 2)

    def test_missing_implementation_measurement_fails_with_reports_retained(self):
        standard = self.source(STANDARD / "sample.py")
        self.record("standard", {str(standard.relative_to(self.root)): [1, 2, 4]})
        with self.assertRaisesRegex(RuntimeError, "generated-implementations"):
            self.report(require_generated=True)
        self.assertEqual(self.read_report()["totals"]["covered_lines"], 3)

    def test_zero_hit_implementation_family_does_not_satisfy_presence_check(self):
        standard = self.source(STANDARD / "sample.py")
        implementation = self.source(IMPLEMENTATIONS / "sample.py")
        self.record("measured", {
            str(standard.relative_to(self.root)): [1, 2, 4],
            str(implementation.relative_to(self.root)): [],
        })
        with self.assertRaisesRegex(RuntimeError, "generated-implementations"):
            self.report(require_generated=True)
        self.assertEqual(
            self.read_report("generated-implementations/coverage.json")["totals"]["covered_lines"],
            0,
        )

    def test_skip_e2e_can_report_runtime_without_generated_sources(self):
        runtime = self.source(Path("bindings/py/src/implementation.py"))
        self.record("runtime", {str(runtime.relative_to(self.root)): [1, 2, 4]})
        self.report()
        self.assertEqual(self.read_report("runtime/coverage.json")["totals"]["covered_lines"], 3)

    def test_installed_runtime_package_is_included(self):
        runtime = self.source(Path("environment/Lib/site-packages/dynwinrt/implementation.py"))
        self.record("runtime", {str(runtime): [1, 2, 4]})
        self.report()
        self.assertEqual(self.read_report("runtime/coverage.json")["totals"]["covered_lines"], 3)

    def test_combined_branch_reports_keep_alias_hits_and_contexts(self):
        source = self.source(
            IMPLEMENTATIONS / "conditional.py",
            "flag = True\nif flag:\n    value = 1\nelse:\n    value = 2\n",
        )
        for context, filename, target in (
            ("truthy", str(source.relative_to(self.root)), 3),
            ("falsey", "\\\\?\\" + str(source), 5),
        ):
            data = CoverageData(basename=str(self.output / (".coverage." + context)))
            data.set_context(context)
            data.add_arcs({filename: [(-1, 1), (1, 2), (2, target), (target, -1)]})
            data.write()
        self.report()
        totals = self.read_report()["totals"]
        self.assertEqual(totals["num_branches"], 2)
        self.assertEqual(totals["covered_branches"], 2)
        self.assertEqual(totals["covered_lines"], 4)
        combined = CoverageData(basename=str(self.output / ".coverage"))
        combined.read()
        self.assertEqual(len(combined.measured_files()), 1)
        filename = next(iter(combined.measured_files()))
        self.assertEqual(set(combined.contexts_by_lineno(filename)[2]), {"truthy", "falsey"})

    def test_no_product_measurements_still_produce_accurate_diagnostics(self):
        runner = self.source(Path("tests/e2e/runners/runner.py"))
        self.record("runner", {str(runner.relative_to(self.root)): [1, 2, 4]})
        self.report()
        self.assertEqual(self.read_report("all-data.json")["totals"]["covered_lines"], 3)
        self.assertFalse((self.output / "coverage.json").exists())

    def test_missing_sources_are_not_ignored(self):
        self.record("missing", {str(STANDARD / "missing.py"): [1]})
        original_directory = Path.cwd()
        with self.assertRaises(NoSource):
            self.report()
        self.assertEqual(Path.cwd(), original_directory)

    def test_extended_long_source_paths_remain_reportable(self):
        long_source = self.source(IMPLEMENTATIONS / ("generic_" + "type_" * 28 + ".py"))
        self.record("long", {"\\\\?\\" + str(long_source): [1, 2, 4]})
        self.report()
        self.assertEqual(self.read_report()["totals"]["covered_lines"], 3)


if __name__ == "__main__":
    unittest.main()
