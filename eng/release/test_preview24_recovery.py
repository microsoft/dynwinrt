# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

import argparse
import io
import json
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest
from unittest.mock import patch

import yaml

import preview24_recovery as recovery


def evidence():
    stages = {
        "Build": "succeeded", "Wait_Python": "succeeded", "Release_GitHub": "succeeded",
        "Collect_Python": "failed", "Release_Npm": "skipped", "Release_PyPI": "skipped",
    }
    names = ["validate", "assemble-release", "github-release"]
    for arch in ("x64", "arm64"):
        names.append(f"consume-codegen-{arch}")
        names.extend(f"consume-runtime-{arch} (3.{minor}, cp3{minor})" for minor in range(11, 15))
    return [
        {
            "id": recovery.BUILD_ID, "definition": {"id": recovery.DEFINITION_ID},
            "project": {"id": recovery.PROJECT},
            "repository": {"id": recovery.REPOSITORY, "type": "GitHub"},
            "sourceBranch": f"refs/tags/{recovery.TAG}", "sourceVersion": recovery.SOURCE_SHA,
            "status": "completed", "result": "failed",
        },
        {"records": [
            {"identifier": name, "type": "Stage", "state": "completed", "result": result}
            for name, result in stages.items()
        ]},
        {"value": [{
            "name": "npm-packages", "id": recovery.NPM_ARTIFACT_ID,
            "resource": {"type": "PipelineArtifact", "data": recovery.NPM_ARTIFACT_DATA},
        }]},
        {"ref": f"refs/tags/{recovery.TAG}", "object": {"type": "commit", "sha": recovery.SOURCE_SHA}},
        {
            "id": recovery.RELEASE_ID, "tag_name": recovery.TAG,
            "target_commitish": recovery.SOURCE_SHA, "draft": False, "prerelease": True,
            "assets": [
                {"name": name, "id": asset_id, "digest": f"sha256:{digest}"}
                for name, (asset_id, digest) in recovery.ASSETS.items()
            ],
        },
        {
            "id": recovery.PYTHON_RUN_ID, "head_sha": recovery.SOURCE_SHA,
            "head_branch": recovery.TAG, "event": "push",
            "path": ".github/workflows/python-release.yml",
            "repository": {"full_name": recovery.REPOSITORY},
            "status": "completed", "conclusion": "success",
        },
        {"jobs": [{"name": name, "status": "completed", "conclusion": "success"} for name in names]},
    ]


class RecoveryTests(unittest.TestCase):
    def test_confirmed_incident_is_accepted(self):
        recovery.validate_evidence(*evidence())

    def test_mismatched_provenance_fails_closed(self):
        changes = [
            (0, ("id",), 123),
            (0, ("definition", "id"), 123),
            (0, ("project", "id"), "wrong-project"),
            (0, ("repository", "id"), "wrong-repo"),
            (0, ("repository", "type"), "TfsGit"),
            (0, ("sourceBranch",), "refs/heads/main"),
            (0, ("sourceVersion",), "f" * 40),
            (0, ("status",), "inProgress"),
            (0, ("result",), "succeeded"),
            (1, ("records", 0, "result"), "failed"),
            (1, ("records", 1, "result"), "skipped"),
            (1, ("records", 4, "result"), "succeeded"),
            (1, ("records", 5, "result"), "failed"),
            (2, ("value",), []),
            (2, ("value", 0, "id"), 123),
            (2, ("value", 0, "resource", "data"), "changed"),
            (3, ("object", "sha"), "f" * 40),
            (3, ("object", "type"), "tag"),
            (4, ("id",), 123),
            (4, ("tag_name",), "v0.1.0-preview.25"),
            (4, ("target_commitish",), "f" * 40),
            (4, ("draft",), True),
            (4, ("prerelease",), False),
            (4, ("assets", 0, "id"), 123),
            (4, ("assets", 0, "digest"), "sha256:" + "f" * 64),
            (4, ("assets",), []),
            (5, ("id",), 123),
            (5, ("head_sha",), "f" * 40),
            (5, ("head_branch",), "main"),
            (5, ("event",), "workflow_dispatch"),
            (5, ("path",), ".github/workflows/build.yml"),
            (5, ("repository", "full_name"), "wrong-repo"),
            (5, ("conclusion",), "failure"),
        ]
        for index, keys, value in changes:
            with self.subTest(keys=keys, value=value):
                data = evidence()
                target = data[index]
                for key in keys[:-1]:
                    target = target[key]
                target[keys[-1]] = value
                with self.assertRaises(ValueError):
                    recovery.validate_evidence(*data)

    def test_every_required_python_job_must_pass(self):
        for index in range(len(evidence()[-1]["jobs"])):
            for conclusion in ("skipped", "failure", None):
                with self.subTest(index=index, conclusion=conclusion):
                    data = evidence()
                    data[-1]["jobs"][index]["conclusion"] = conclusion
                    with self.assertRaisesRegex(ValueError, "Required tested Python job"):
                        recovery.validate_evidence(*data)
            data = evidence()
            data[-1]["jobs"].pop(index)
            with self.assertRaisesRegex(ValueError, "Required tested Python job"):
                recovery.validate_evidence(*data)

    def test_context_distinguishes_recovery_and_package_commits(self):
        context = {
            "BUILD_REASON": "Manual", "BUILD_SOURCEBRANCH": recovery.BRANCH,
            "BUILD_SOURCEVERSION": "a" * 40,
        }
        self.assertEqual(recovery.validate_context(context), "a" * 40)
        for key, value in (
            ("BUILD_REASON", "IndividualCI"),
            ("BUILD_SOURCEBRANCH", f"refs/tags/{recovery.TAG}"),
            ("BUILD_SOURCEBRANCH", "refs/heads/main"),
            ("BUILD_SOURCEVERSION", recovery.SOURCE_SHA),
            ("BUILD_SOURCEVERSION", "not-a-sha"),
        ):
            with self.subTest(key=key, value=value), self.assertRaises(ValueError):
                recovery.validate_context({**context, key: value})

    def test_read_only_api_checks_keep_tokens_on_their_own_hosts(self):
        with patch.dict(recovery.os.environ, {
            "SYSTEM_ACCESSTOKEN": "ado-test-token", "GH_TOKEN": "github-test-token",
        }, clear=True), patch.object(recovery, "get_json", side_effect=evidence()) as get:
            recovery.validate_remote()
            self.assertEqual(get.call_count, 7)
            for call in get.call_args_list[:3]:
                self.assertTrue(call.args[0].startswith("https://dev.azure.com/microsoft/"))
                self.assertEqual(call.args[1], "ado-test-token")
            for call in get.call_args_list[3:]:
                self.assertTrue(call.args[0].startswith("https://api.github.com/repos/microsoft/dynwinrt/"))
                self.assertEqual(call.args[1], "github-test-token")

    def test_package_sets_reject_missing_extra_duplicate_and_changed_bytes(self):
        with tempfile.TemporaryDirectory(prefix="preview24-files-") as temporary:
            root = Path(temporary)
            for suffix in (".whl", ".tgz"):
                with self.subTest(suffix=suffix):
                    with self.assertRaisesRegex(ValueError, "Package set mismatch"):
                        recovery.verify_files(root, suffix)
                    files = [root / name for name in recovery.ASSETS if name.endswith(suffix)]
                    for path in files:
                        path.write_bytes(b"wrong bytes")
                    with self.assertRaisesRegex(ValueError, "SHA256 mismatch"):
                        recovery.verify_files(root, suffix)
                    extra = root / f"unexpected{suffix}"
                    extra.touch()
                    with self.assertRaisesRegex(ValueError, "Package set mismatch"):
                        recovery.verify_files(root, suffix)
                    extra.unlink()
                    nested = root / suffix[1:]
                    nested.mkdir()
                    (nested / files[0].name).touch()
                    with self.assertRaisesRegex(ValueError, "Package set mismatch"):
                        recovery.verify_files(root, suffix)

    def test_npm_manifest_name_and_version(self):
        with tempfile.TemporaryDirectory(prefix="preview24-npm-") as temporary:
            path = Path(temporary) / f"microsoft-dynwinrt-{recovery.VERSION}.tgz"
            for name, version, valid in (
                ("@microsoft/dynwinrt", recovery.VERSION, True),
                ("@microsoft/dynwinrt-codegen", recovery.VERSION, False),
                ("@microsoft/dynwinrt", "0.1.0-preview.25", False),
            ):
                payload = json.dumps({"name": name, "version": version}).encode()
                with tarfile.open(path, "w:gz") as archive:
                    info = tarfile.TarInfo("package/package.json")
                    info.size = len(payload)
                    archive.addfile(info, io.BytesIO(payload))
                if valid:
                    recovery.verify_npm_metadata(path)
                else:
                    with self.assertRaisesRegex(ValueError, "npm name/version mismatch"):
                        recovery.verify_npm_metadata(path)

    def test_complete_release_validator_is_mandatory(self):
        with patch.object(recovery, "verify_files", return_value=[]), \
                patch.object(recovery, "verify_release_set", side_effect=ValueError("bad wheel")) as verify:
            with self.assertRaisesRegex(ValueError, "bad wheel"):
                recovery.verify_packages(Path("npm"), Path("wheels"))
            verify.assert_called_once_with(
                argparse.Namespace(directory=Path("wheels"), version=recovery.VERSION)
            )


class PipelineTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.pipeline = yaml.safe_load((recovery.ROOT / ".pipelines" / "release.yml").read_text())
        cls.original = yaml.safe_load(subprocess.check_output(
            ["git", "show", f"{recovery.SOURCE_SHA}:.pipelines/release.yml"],
            cwd=recovery.ROOT, text=True,
        ))
        cls.stages = cls.pipeline["extends"]["parameters"]["stages"]
        cls.collect = cls.stages[0]
        cls.steps = cls.collect["jobs"][0]["steps"]

    def test_manual_only_no_build_or_github_mutation(self):
        self.assertEqual(self.pipeline["trigger"], "none")
        self.assertEqual(self.pipeline["pr"], "none")
        self.assertEqual(self.collect["condition"], "and(succeeded(), eq(variables['Build.Reason'], 'Manual'))")
        parameters = {param["name"]: param for param in self.pipeline["parameters"]}
        self.assertEqual(set(parameters), {"DoEsrp", "PublishPyPI", "signingIdentity"})
        self.assertIs(parameters["DoEsrp"]["default"], False)
        self.assertIs(parameters["PublishPyPI"]["default"], True)
        self.assertEqual(len(self.stages), 3)
        tasks = [step.get("task") for step in self.steps]
        self.assertNotIn("GitHubRelease@1", tasks)
        self.assertNotIn("EsrpRelease@10", tasks)
        self.assertEqual(tasks.count("1ES.PublishPipelineArtifact@1"), 2)
        self.assertEqual(self.steps[0], {"checkout": "self", "fetchDepth": 0})
        self.assertEqual(self.collect["jobs"][0]["workspace"], {"clean": "all"})

    def test_fixed_downloads_and_python_setup_mirror(self):
        tasks = {step["task"]: step for step in self.steps if "task" in step}
        npm = tasks["DownloadPipelineArtifact@2"]["inputs"]
        self.assertEqual(npm["buildType"], "specific")
        self.assertEqual(npm["buildVersionToDownload"], "specific")
        self.assertEqual(npm["pipelineId"], str(recovery.BUILD_ID))
        self.assertEqual(npm["definition"], str(recovery.DEFINITION_ID))
        self.assertEqual(npm["project"], recovery.PROJECT)
        self.assertIs(npm["allowFailedBuilds"], True)
        self.assertEqual(npm["artifactName"], "npm-packages")
        wheels = tasks["DownloadGitHubRelease@0"]["inputs"]
        self.assertEqual(wheels["version"], recovery.TAG)
        self.assertEqual(wheels["defaultVersionType"], "specificTag")
        setup = tasks["UsePythonVersion@0"]
        self.assertEqual(setup["env"]["PIP_INDEX_URL"], "https://packagefeedproxy.microsoft.io/pypi/simple/")
        verify = tasks["PowerShell@2"]
        self.assertLess(self.steps.index(setup), self.steps.index(verify))
        self.assertEqual(verify["name"], "SetMeta")
        self.assertIn("python eng\\release\\preview24_recovery.py", verify["inputs"]["script"])
        self.assertEqual(verify["env"]["SYSTEM_ACCESSTOKEN"], "$(System.AccessToken)")
        for step in self.steps[self.steps.index(verify) + 1:]:
            self.assertEqual(step["task"], "1ES.PublishPipelineArtifact@1")

    def test_esrp_jobs_approvals_and_publication_order_are_preserved(self):
        self.assertEqual(self.pipeline["resources"], self.original["resources"])
        self.assertEqual(self.pipeline["variables"], self.original["variables"])
        self.assertEqual(self.pipeline["extends"]["template"], self.original["extends"]["template"])
        original_stages = self.original["extends"]["parameters"]["stages"]
        for current, original in zip(self.stages[1:], original_stages[-2:]):
            self.assertEqual(set(current), set(original))
            stage = next(iter(current.values()))[0]
            original_stage = next(iter(original.values()))[0]
            self.assertEqual(stage["jobs"], original_stage["jobs"])
            self.assertEqual(
                stage["dependsOn"], ["Collect_Python"]
                + (["Release_Npm"] if stage["stage"] == "Release_PyPI" else []),
            )
            self.assertNotIn("condition", stage)
            self.assertEqual(stage["variables"]["version"],
                             "$[ stageDependencies.Collect_Python.collect_python.outputs['SetMeta.version'] ]")
        self.assertEqual(
            self.pipeline["parameters"][-1], self.original["parameters"][-1]
        )


if __name__ == "__main__":
    unittest.main()
