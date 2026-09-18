# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import copy
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import tempfile
import unittest

import yaml

from classify_changes import classify, documentation_diff


ROOT = Path(__file__).resolve().parents[2]
CI = ROOT / "eng" / "ci"
WORKFLOW = yaml.safe_load((ROOT / ".github" / "workflows" / "build.yml").read_text(encoding="utf-8"))
JOBS = WORKFLOW["jobs"]
HEAVY = {
    "test-rust", "build-codegen-x64", "build-bindings-x64", "build-js-test-hooks",
    "build-arm64", "e2e-runtime", "e2e-native",
}
GATES = {"test", "e2e", "dynwinrt", "dynwinrt-codegen", "ci-success"}


def powershell(script, *arguments, env=None):
    return subprocess.run(
        ["pwsh", "-NoProfile", "-File", str(script), *map(str, arguments)],
        cwd=ROOT, env=env, text=True, encoding="utf-8", capture_output=True,
    )


class ClassificationTests(unittest.TestCase):
    def test_markdown_paths_default_to_lightweight(self):
        for path in (
            "README.md", "new/directory/new.md", "docs/guides/new-guide.md",
            "samples/new/README.md", "bindings/js/README.md", "bindings/py/README.md",
            "tools/dynwinrt-codegen/npm/README.md", "tools/dynwinrt-codegen/python/README.md",
            "eng/release/RELEASE_NOTES.md", "docs/status/TODO.md",
            "docs/status/generated/classic-com-named-types.md", "docs/status/future-report.md",
            "docs/UPPER.MD", "docs/Mixed.mD", "docs/with spaces.md", "docs/with\nnewline.md",
        ):
            for status in ("A", "M", "D"):
                with self.subTest(path=path, status=status):
                    self.assertTrue(documentation_diff(f"{status}\0{path}\0".encode()))
        self.assertTrue(documentation_diff(b"M\0README.md\0A\0future/new.md\0"))

    def test_non_markdown_and_mixed_changes_need_full_validation(self):
        for path in (
            "docs/status/generated/classic-com-capability-summary.json",
            "docs/status/generated/classic-com-interface-support.csv",
            "contracts/schema/contract.schema.json", "tests/e2e/e2e_specs.schema.json",
            "eng/ci/classify_changes.py", ".github/workflows/build.yml",
            "samples/js/ocr/main.ts", "samples/js/ocr/package.json", "crates/dynwinrt/src/lib.rs",
            "future/unknown.data", "docs/guide.markdown", "docs/guide.mdx", "README.md.bak",
            "notesmd", "",
        ):
            with self.subTest(path=path):
                self.assertFalse(documentation_diff(f"A\0{path}\0".encode()))
                self.assertFalse(documentation_diff(f"M\0README.md\0M\0{path}\0".encode()))

    def test_deletions_renames_unknown_status_and_empty_diff(self):
        self.assertTrue(documentation_diff(b"D\0README.md\0"))
        self.assertTrue(documentation_diff(b"R100\0README.md\0samples/js/README.md\0"))
        self.assertTrue(documentation_diff(b"R075\0old/guide.MD\0new/guide.md\0"))
        self.assertFalse(documentation_diff(b"R100\0src/main.rs\0README.md\0"))
        self.assertFalse(documentation_diff(b"R100\0README.md\0src/main.rs\0"))
        self.assertFalse(documentation_diff(b"D\0src/main.rs\0A\0README.md\0"))
        for status in ("T", "U", "X", "C100", "R", "R101", "R1000", "R-1"):
            with self.subTest(status=status):
                self.assertFalse(documentation_diff(f"{status}\0README.md\0new.md\0".encode()))
        self.assertFalse(documentation_diff(b""))
        for malformed in (b"M\0README.md", b"R100\0README.md\0", b"M\0"):
            with self.subTest(diff=malformed), self.assertRaises(ValueError):
                documentation_diff(malformed)

    def test_entire_pr_history_and_missing_history(self):
        with tempfile.TemporaryDirectory(prefix="dynwinrt-ci-git-") as directory:
            repo = Path(directory)

            def git(*args):
                return subprocess.check_output(["git", *args], cwd=repo).decode().strip()

            git("init", "--quiet")
            git("config", "user.name", "CI test")
            git("config", "user.email", "ci@example.invalid")
            (repo / "README.md").write_text("base\n")
            (repo / "code.rs").write_text("base\n")
            git("add", ".")
            git("commit", "--quiet", "-m", "base")
            base = git("rev-parse", "HEAD")
            (repo / "code.rs").write_text("changed\n")
            git("commit", "--quiet", "-am", "code in earlier PR commit")
            (repo / "README.md").write_text("docs in latest commit\n")
            git("commit", "--quiet", "-am", "docs")
            event = {"pull_request": {"base": {"sha": base}, "head": {"sha": git("rev-parse", "HEAD")}}}
            self.assertFalse(classify("pull_request", event, repo))
            # The latest commit alone is docs-only, but the full PR above is not.
            event["pull_request"]["base"]["sha"] = git("rev-parse", "HEAD~1")
            self.assertTrue(classify("pull_request", event, repo))
            (repo / "new").mkdir()
            (repo / "new" / "guide.MD").write_text("new Markdown\n")
            git("add", ".")
            git("commit", "--quiet", "-m", "new Markdown directory")
            event["pull_request"]["head"]["sha"] = git("rev-parse", "HEAD")
            self.assertTrue(classify("pull_request", event, repo))
            event["pull_request"]["base"]["sha"] = base
            self.assertFalse(classify("pull_request", event, repo))
            self.assertFalse(classify("push", event, repo))
            self.assertFalse(classify("workflow_dispatch", event, repo))
            event["pull_request"]["head"]["sha"] = "f" * 40
            with self.assertRaises(subprocess.CalledProcessError):
                classify("pull_request", event, repo)
            event["pull_request"]["head"]["sha"] = "--help"
            with self.assertRaises(ValueError):
                classify("pull_request", event, repo)


class PackageSourceTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="dynwinrt-ci-source-")
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name)
        for relative in (
            "bindings/py/Cargo.toml", "bindings/py/pyproject.toml", "bindings/py/README.md",
            "tools/dynwinrt-codegen/Cargo.toml", "tools/dynwinrt-codegen/pyproject.toml",
            "tools/dynwinrt-codegen/python/README.md",
            "tools/dynwinrt-codegen/src/codegen/package.rs",
            "eng/release/python/verify_python_release.py",
        ):
            target = self.repo / relative
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copyfile(ROOT / relative, target)
        steps = [
            step for step in JOBS["format"]["steps"]
            if step.get("name") == "Validate Python package source metadata"
        ]
        self.assertEqual(len(steps), 1)
        self.assertNotIn("if", JOBS["format"])
        self.assertNotIn("if", steps[0])
        self.assertEqual(steps[0]["shell"], "pwsh")
        script = self.repo / "step.ps1"
        script.write_text(
            "$ErrorActionPreference = 'Stop'\n"
            + steps[0]["run"]
            + "\nif (Test-Path -LiteralPath variable:\\LASTEXITCODE) { exit $LASTEXITCODE }\n",
            encoding="utf-8",
        )
        quoted_script = str(script).replace("'", "''")
        self.command = f". '{quoted_script}'"

    def run_step(self):
        return subprocess.run(
            ["pwsh", "-NoLogo", "-NoProfile", "-NonInteractive", "-Command", self.command],
            cwd=self.repo, text=True, encoding="utf-8", capture_output=True,
        )

    def test_source_step_succeeds_in_github_wrapper(self):
        result = self.run_step()
        self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("source metadata OK:", result.stdout)

    def test_metadata_failure_reaches_github_wrapper(self):
        manifest = self.repo / "bindings" / "py" / "Cargo.toml"
        original = manifest.read_text(encoding="utf-8")
        version = next(line for line in original.splitlines() if line.startswith("version = "))
        manifest.write_text(original.replace(version, 'version = "999.0.0"', 1), encoding="utf-8")
        result = self.run_step()
        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        self.assertIn("Runtime and codegen Cargo versions must match", result.stdout + result.stderr)

    def test_missing_empty_and_invalid_readmes_fail_in_github_wrapper(self):
        for relative in ("bindings/py/README.md", "tools/dynwinrt-codegen/python/README.md"):
            readme = self.repo / relative
            original = readme.read_bytes()
            for contents in (None, "", "# Unrelated package\n"):
                with self.subTest(readme=relative, contents=contents):
                    try:
                        if contents is None:
                            readme.unlink()
                        else:
                            readme.write_text(contents, encoding="utf-8")
                        result = self.run_step()
                        self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
                        self.assertIn("Package README", result.stdout + result.stderr)
                    finally:
                        readme.write_bytes(original)


class WorkflowTests(unittest.TestCase):
    def ancestors(self, job):
        needs = JOBS[job].get("needs", [])
        if isinstance(needs, str):
            needs = [needs]
        return set(needs).union(*(self.ancestors(parent) for parent in needs))

    def test_graph_and_parallel_consumers(self):
        self.assertEqual(WORKFLOW["permissions"], {"contents": "read"})
        # PyYAML uses YAML 1.1: an unquoted "on" key is represented by True.
        triggers = WORKFLOW.get("on", WORKFLOW.get(True))
        self.assertEqual(set(triggers), {"push", "pull_request"})
        self.assertNotIn("paths-ignore", triggers["pull_request"])
        self.assertEqual(JOBS["changes"]["steps"][0]["with"]["fetch-depth"], 0)
        for job in HEAVY:
            self.assertEqual(JOBS[job]["if"], "needs.changes.outputs.docs_only == 'false'")
        for producer in ("build-codegen-x64", "build-bindings-x64", "build-js-test-hooks", "build-arm64"):
            self.assertEqual(set(JOBS[producer]["needs"]), {"format", "changes"})
        for consumer in ("e2e-runtime", "e2e-native"):
            self.assertTrue(self.ancestors(consumer).isdisjoint({"test", "test-rust", "build-arm64"}))
        for final in ("dynwinrt", "dynwinrt-codegen"):
            self.assertTrue({"test", "e2e", "build-arm64"} <= self.ancestors(final))
        self.assertEqual(set(JOBS["ci-success"]["needs"]), set(JOBS) - {"ci-success"})

    def test_current_run_artifacts_and_no_consumer_rebuilds(self):
        uploads = {}
        for job, config in JOBS.items():
            for step in config["steps"]:
                action = step.get("uses", "")
                if action.startswith("actions/upload-artifact@"):
                    name = step["with"]["name"]
                    self.assertNotIn(name, uploads)
                    self.assertEqual(step["with"]["if-no-files-found"], "error")
                    uploads[name] = job
        for job, config in JOBS.items():
            for step in config["steps"]:
                if step.get("uses", "").startswith("actions/download-artifact@"):
                    inputs = step["with"]
                    self.assertEqual(set(inputs), {"name", "path"})
                    producer = uploads[inputs["name"]]
                    self.assertTrue(producer == job or producer in self.ancestors(job))
        self.assertEqual(uploads["dynwinrt"], "dynwinrt")
        self.assertEqual(uploads["dynwinrt-codegen"], "dynwinrt-codegen")
        for job in ("e2e-runtime", "e2e-native", "dynwinrt", "dynwinrt-codegen"):
            commands = "\n".join(step.get("run", "") for step in JOBS[job]["steps"])
            for forbidden in ("cargo build", "cargo run", "napi build", "maturin build", "npm run test:tsfn",
                              "npm run test:generated-unsafe", "npm run test:native-completion",
                              "npm run test:borrowed-copy"):
                self.assertNotIn(forbidden, commands)
        runtime = "\n".join(step.get("run", "") for step in JOBS["e2e-runtime"]["steps"])
        self.assertIn("cargo test -p dynwinrt-codegen --test implementation_naming_test", runtime)

    def test_release_notes_validated_in_lightweight_lane(self):
        steps = [
            step for step in JOBS["format"]["steps"]
            if step.get("name") == "Validate checked-in release notes"
        ]
        self.assertEqual(len(steps), 1)
        self.assertNotIn("if", JOBS["format"])
        self.assertNotIn("if", steps[0])
        self.assertEqual(steps[0]["shell"], "pwsh")
        self.assertEqual(steps[0]["run"].splitlines(), [
            r".\eng\release\validate_release_notes.ps1",
            r".\eng\release\test_validate_release_notes.ps1",
        ])

    def test_gate_truth_tables_execute_the_real_gate(self):
        # Run the entire matrix in one PowerShell process, not hundreds of shells.
        cases = []
        for job in GATES:
            config = JOBS[job]
            self.assertEqual(config["if"], "${{ always() }}")
            gate_index, gate = next(
                (index, step) for index, step in enumerate(config["steps"])
                if "assert-success.ps1" in step.get("run", "")
            )
            env = gate["env"]
            required = env["REQUIRED_JOBS"].split(",")
            skipped = env["DOCS_SKIPPABLE_JOBS"].split(",")
            self.assertEqual(set(required), set(config["needs"]))
            self.assertEqual(set(skipped), HEAVY & set(required))
            self.assertEqual(env["DOCS_ONLY"], "${{ needs.changes.outputs.docs_only }}")
            if job in {"dynwinrt", "dynwinrt-codegen"}:
                for step in config["steps"][gate_index + 1:]:
                    self.assertEqual(step["if"], "needs.changes.outputs.docs_only == 'false'")
            for docs in (False, True):
                needs = {name: {"result": "skipped" if docs and name in skipped else "success"} for name in required}
                needs["changes"]["outputs"] = {"docs_only": str(docs).lower()}
                case = {"needs": needs, "required": required, "skipped": skipped, "docs": str(docs).lower(), "ok": True}
                cases.append(case)
                for dependency in required:
                    for result in ("success", "failure", "cancelled", "skipped", ""):
                        if result == needs[dependency]["result"]:
                            continue
                        broken = copy.deepcopy(case)
                        broken["needs"][dependency]["result"] = result
                        broken["ok"] = False
                        cases.append(broken)
                missing = copy.deepcopy(case)
                del missing["needs"][required[-1]]
                missing["ok"] = False
                cases.append(missing)
                unknown = copy.deepcopy(case)
                unknown["needs"]["unexpected"] = {"result": "success"}
                unknown["ok"] = False
                cases.append(unknown)
                inconsistent = copy.deepcopy(case)
                inconsistent["needs"]["changes"]["outputs"]["docs_only"] = str(not docs).lower()
                inconsistent["ok"] = False
                cases.append(inconsistent)
        with tempfile.TemporaryDirectory(prefix="dynwinrt-ci-gates-") as directory:
            cases_path = Path(directory) / "cases.json"
            cases_path.write_text(json.dumps(cases))
            harness = Path(directory) / "gates.ps1"
            harness.write_text(
                'param([string]$Gate, [string]$Cases)\n'
                '$ErrorActionPreference = "Stop"\n'
                '$index = 0\n'
                'foreach ($case in (Get-Content $Cases -Raw | ConvertFrom-Json)) {\n'
                '  $passed = $true\n'
                '  try {\n'
                '    & $Gate -NeedsJson ($case.needs | ConvertTo-Json -Depth 5 -Compress) '
                '-RequiredJobs $case.required -DocsOnly $case.docs -SkippableJobs $case.skipped | Out-Null\n'
                '  } catch { $passed = $false }\n'
                '  if ($passed -ne $case.ok) { throw "Gate truth-table case ${index}: expected $($case.ok)" }\n'
                '  $index++\n'
                '}\n',
                encoding="utf-8",
            )
            result = powershell(harness, CI / "assert-success.ps1", cases_path)
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)


class ArtifactTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory(prefix="dynwinrt-ci-artifact-")
        self.addCleanup(self.temporary.cleanup)
        self.root = Path(self.temporary.name)
        self.env = {**os.environ, "GITHUB_SHA": "1" * 40, "GITHUB_RUN_ID": "12345"}

    def invoke(self, mode, path, kind="js", arch="x64", *extra, ok=True):
        result = powershell(
            CI / "artifact.ps1", "-Mode", mode, "-Path", path, "-Kind", kind,
            "-Architecture", arch, *extra, env=self.env,
        )
        self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)

    def payload(self, name, arch="x64", kind="js"):
        artifact = self.root / name
        payload = artifact / "payload"
        payload.mkdir(parents=True)
        native = payload / ("dynwinrt-codegen.exe" if kind == "codegen" else f"dynwinrt.win32-{arch}-msvc.node")
        image = bytearray(128)
        struct.pack_into("<H", image, 0, 0x5A4D)
        struct.pack_into("<I", image, 0x3C, 64)
        struct.pack_into("<IH", image, 64, 0x4550, 0x8664 if arch == "x64" else 0xAA64)
        native.write_bytes(image)
        if kind == "js":
            (payload / "index.js").write_text("same architecture-independent sidecar\n")
        return artifact

    def test_identity_hashes_architecture_and_missing_files_fail_closed(self):
        artifact = self.payload("js")
        self.invoke("Write", artifact)
        self.invoke("Verify", artifact)
        original = (artifact / "manifest.json").read_bytes()
        for key, value in (
            ("commit", "2" * 40), ("run", "other-run"), ("kind", "codegen"),
            ("version", "9.9.9"), ("target", "aarch64-pc-windows-msvc"),
            ("profile", "coverage"), ("features", "test-hooks"), ("pythonAbi", "cp313"),
        ):
            with self.subTest(key=key):
                manifest = json.loads(original)
                manifest[key] = value
                (artifact / "manifest.json").write_text(json.dumps(manifest))
                self.invoke("Verify", artifact, ok=False)
        (artifact / "manifest.json").write_bytes(original)
        sidecar = artifact / "payload" / "index.js"
        sidecar.write_text("tampered\n")
        self.invoke("Verify", artifact, ok=False)
        sidecar.unlink()
        self.invoke("Verify", artifact, ok=False)
        (artifact / "manifest.json").unlink()
        self.invoke("Verify", artifact, ok=False)
        self.invoke("Verify", self.root / "missing", ok=False)
        wrong_arch = self.payload("bad-architecture")
        shutil.copyfile(
            self.payload("arm64", arch="arm64") / "payload" / "dynwinrt.win32-arm64-msvc.node",
            wrong_arch / "payload" / "dynwinrt.win32-x64-msvc.node",
        )
        self.invoke("Write", wrong_arch, ok=False)

    def test_exact_python_wheel_and_codegen_contracts(self):
        python = self.root / "python"
        (python / "payload").mkdir(parents=True)
        version = next(
            line.split('"')[1] for line in (ROOT / "bindings" / "py" / "Cargo.toml").read_text().splitlines()
            if line.startswith("version = ")
        )
        wheel = python / "payload" / f"dynwinrt-{version}-cp312-cp312-win_amd64.whl"
        wheel.write_bytes(b"wheel fixture; pip performs wheel validation during consumption")
        args = ("-Profile", "dev", "-PythonAbi", "cp312")
        self.invoke("Write", python, "python", "x64", *args)
        self.invoke("Verify", python, "python", "x64", *args)
        wheel.rename(wheel.with_name(wheel.name.replace("cp312", "cp313")))
        self.invoke("Verify", python, "python", "x64", *args, ok=False)
        codegen = self.payload("codegen", kind="codegen")
        self.invoke("Write", codegen, "codegen")
        self.invoke("Verify", codegen, "codegen")

    def test_merge_requires_identical_sidecars_and_production_features(self):
        x64 = self.payload("x64")
        arm64 = self.payload("arm64", arch="arm64")
        for path, arch in ((x64, "x64"), (arm64, "arm64")):
            self.invoke("Write", path, arch=arch)
        output = self.root / "assembled"

        def merge(ok):
            result = powershell(
                CI / "merge-runtime-artifacts.ps1", "-X64", x64, "-Arm64", arm64,
                "-Output", output, env=self.env,
            )
            self.assertEqual(result.returncode == 0, ok, result.stdout + result.stderr)

        merge(True)
        self.assertEqual({file.name for file in output.iterdir()}, {
            "index.js", "dynwinrt.win32-x64-msvc.node", "dynwinrt.win32-arm64-msvc.node",
        })
        merge(False)
        shutil.rmtree(output)
        (arm64 / "payload" / "index.js").write_text("different, but correctly hashed\n")
        (arm64 / "manifest.json").unlink()
        self.invoke("Write", arm64, arch="arm64")
        merge(False)
        self.assertFalse(output.exists())
        (arm64 / "manifest.json").unlink()
        self.invoke("Write", arm64, "js", "arm64", "-Features", "test-hooks")
        merge(False)


if __name__ == "__main__":
    unittest.main()
