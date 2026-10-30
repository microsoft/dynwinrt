# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Validate and stage existing preview.24 packages; never build or publish."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tarfile
from urllib.request import Request, urlopen

sys.path.insert(0, str(Path(__file__).resolve().parent / "python"))
from verify_python_release import verify_release_set


ROOT = Path(__file__).resolve().parents[2]
REPOSITORY = "microsoft/dynwinrt"
BRANCH = "refs/heads/lei9444-release-preview24-recovery"
VERSION = "0.1.0-preview.24"
TAG = f"v{VERSION}"
SOURCE_SHA = "1ab0b31b6e59ef2a44215a5f394d90eed8fdcd4b"
PROJECT = "71503134-3b3d-4e75-aa3f-25519f1b7d3f"
BUILD_ID = 162292158
DEFINITION_ID = 191586
NPM_ARTIFACT_ID = 356126797
NPM_ARTIFACT_DATA = "704C7EC3A8B94FDCC3785FEAB89CEC99E89B27AE8F0D6723D399754BFA6D84C201"
RELEASE_ID = 429044377
PYTHON_RUN_ID = 41055989308

# Compared with the original ADO npm artifact and tested GitHub assembly artifact
# 12977716730 before preparing this branch, not merely inferred from filenames.
ASSETS = {
    "dynwinrt-0.1.0rc24-cp311-cp311-win_amd64.whl":
        (692154118, "1fbcadcceba7387bd07c919638d2de616a8e51e2e653deea85d640f540d64fc1"),
    "dynwinrt-0.1.0rc24-cp311-cp311-win_arm64.whl":
        (692154116, "f133b36caf6ae77bb20769ca758a447e380f230e7bbb54f9e3aa23c85704fe9b"),
    "dynwinrt-0.1.0rc24-cp312-cp312-win_amd64.whl":
        (692154113, "0fa3ace86cce881fc302f3d2c1ff1ab3caca5980b288ca191adf6adb6da18150"),
    "dynwinrt-0.1.0rc24-cp312-cp312-win_arm64.whl":
        (692154157, "e7ea1ede177ca291a405c9d3601fff956b56bf757620dce9a4df6b6dedfa6fe5"),
    "dynwinrt-0.1.0rc24-cp313-cp313-win_amd64.whl":
        (692154158, "d5e105d579c6ba7a3a3c5a188879f874a1aeae342d55b20d248eb673b97dd1f0"),
    "dynwinrt-0.1.0rc24-cp313-cp313-win_arm64.whl":
        (692154164, "a11cf3ade01653f73a4bb77c9d6cb1fd655165c72375927634d62d5c24ceeefa"),
    "dynwinrt-0.1.0rc24-cp314-cp314-win_amd64.whl":
        (692154165, "0d168d13cb046852c2ff365fbf355b6d4adba7c7050b2ca23697e3fe04314993"),
    "dynwinrt-0.1.0rc24-cp314-cp314-win_arm64.whl":
        (692154177, "ee79427f4e2eeaa6d83927acfc2579627aea4a196cb9db777b400c4937c832d1"),
    "dynwinrt_codegen-0.1.0rc24-py3-none-win_amd64.whl":
        (692154122, "6d7562929ae20c0e3363b82c75d92c3743bce1ea04aa89f0af0d70e8836662fc"),
    "dynwinrt_codegen-0.1.0rc24-py3-none-win_arm64.whl":
        (692154120, "a1631d8f3d7ce40833e9bd3f73fe53f3519dc1de7963725d984949aa254b6583"),
    "microsoft-dynwinrt-0.1.0-preview.24.tgz":
        (692153980, "30420f63d655967b1b3e0b91b09cb88449076249c6cddf1bc62c71b6a6f09e76"),
    "microsoft-dynwinrt-codegen-0.1.0-preview.24.tgz":
        (692154089, "20e5882345e8f0a8347d5c195ef3bce123191185decf73a85d1a7ec7ce7f064b"),
}
RECOVERY_FILES = (
    ".pipelines/release.yml",
    "eng/release/README.md",
    "eng/release/preview24_recovery.py",
    "eng/release/test_preview24_recovery.py",
)


def require(condition: bool, message: str) -> None:
    if not condition:
        raise ValueError(message)


def get_json(url: str, token: str | None = None) -> dict:
    headers = {"Accept": "application/json", "User-Agent": "dynwinrt-preview24-recovery"}
    if token:
        headers["Authorization"] = f"Bearer {token}"
    with urlopen(Request(url, headers=headers), timeout=60) as response:
        return json.load(response)


def validate_context(environment: dict[str, str]) -> str:
    require(environment.get("BUILD_REASON") == "Manual", "Recovery must be manually queued")
    require(environment.get("BUILD_SOURCEBRANCH") == BRANCH, "Wrong recovery branch")
    recovery_sha = environment.get("BUILD_SOURCEVERSION", "")
    require(
        re.fullmatch(r"[0-9a-f]{40}", recovery_sha) is not None and recovery_sha != SOURCE_SHA,
        "Recovery commit must be distinct from the original package source SHA",
    )
    require(sys.flags.optimize == 0, "Python optimization would disable release-set assertions")
    return recovery_sha


def validate_checkout(recovery_sha: str) -> None:
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    require(head == recovery_sha, "Checked-out commit differs from the recovery run")
    subprocess.run(["git", "merge-base", "--is-ancestor", SOURCE_SHA, "HEAD"], cwd=ROOT, check=True)
    # Keep the authoritative source/stubs and the complete-release validator at
    # the package source commit, even though the orchestration uses a new commit.
    subprocess.run(
        ["git", "diff", "--exit-code", "--quiet", SOURCE_SHA, "--", "."]
        + [f":(exclude){path}" for path in RECOVERY_FILES],
        cwd=ROOT, check=True,
    )


def validate_evidence(build, timeline, artifacts, tag, release, run, jobs) -> None:
    require(
        build["id"] == BUILD_ID
        and build["definition"]["id"] == DEFINITION_ID
        and build["project"]["id"] == PROJECT
        and build["repository"]["id"] == REPOSITORY
        and build["repository"]["type"] == "GitHub"
        and build["sourceBranch"] == f"refs/tags/{TAG}"
        and build["sourceVersion"] == SOURCE_SHA
        and build["status"] == "completed"
        and build["result"] == "failed",
        "Original build/tag/source identity mismatch",
    )
    stages = {
        record["identifier"]: record
        for record in timeline["records"] if record["type"] == "Stage"
    }
    for name, result in (
        ("Build", "succeeded"), ("Wait_Python", "succeeded"),
        ("Release_GitHub", "succeeded"), ("Collect_Python", "failed"),
        ("Release_Npm", "skipped"), ("Release_PyPI", "skipped"),
    ):
        require(
            name in stages and stages[name]["state"] == "completed"
            and stages[name]["result"] == result,
            f"Original stage {name} must have completed as {result}",
        )
    npm = [artifact for artifact in artifacts["value"] if artifact["name"] == "npm-packages"]
    require(
        len(npm) == 1 and npm[0]["id"] == NPM_ARTIFACT_ID
        and npm[0]["resource"]["type"] == "PipelineArtifact"
        and npm[0]["resource"]["data"] == NPM_ARTIFACT_DATA,
        "Original npm pipeline artifact is missing or changed",
    )
    require(
        tag["ref"] == f"refs/tags/{TAG}" and tag["object"]["type"] == "commit"
        and tag["object"]["sha"] == SOURCE_SHA,
        "Original release tag moved or changed",
    )
    require(
        release["id"] == RELEASE_ID and release["tag_name"] == TAG
        and release["target_commitish"] == SOURCE_SHA
        and release["draft"] is False and release["prerelease"] is True,
        "Existing GitHub release identity/state mismatch",
    )
    assets = release["assets"]
    require(
        len(assets) == len(ASSETS) and {asset["name"] for asset in assets} == set(ASSETS),
        "Existing GitHub release asset set changed",
    )
    for asset in assets:
        asset_id, digest = ASSETS[asset["name"]]
        require(
            asset["id"] == asset_id and asset["digest"] == f"sha256:{digest}",
            f"Existing GitHub release asset changed: {asset['name']}",
        )
    require(
        run["id"] == PYTHON_RUN_ID and run["head_sha"] == SOURCE_SHA
        and run["head_branch"] == TAG and run["event"] == "push"
        and run["path"] == ".github/workflows/python-release.yml"
        and run["repository"]["full_name"] == REPOSITORY
        and run["status"] == "completed" and run["conclusion"] == "success",
        "Tested Python workflow identity/result mismatch",
    )
    expected_jobs = {"validate", "assemble-release", "github-release"}
    for arch in ("x64", "arm64"):
        expected_jobs.add(f"consume-codegen-{arch}")
        for minor in range(11, 15):
            expected_jobs.add(f"consume-runtime-{arch} (3.{minor}, cp3{minor})")
    for name in expected_jobs:
        matches = [job for job in jobs["jobs"] if job["name"] == name]
        require(
            len(matches) == 1 and matches[0]["status"] == "completed"
            and matches[0]["conclusion"] == "success",
            f"Required tested Python job did not pass: {name}",
        )


def validate_remote() -> None:
    token = os.environ.get("SYSTEM_ACCESSTOKEN")
    require(bool(token), "SYSTEM_ACCESSTOKEN is required for read-only original-run validation")
    ado = f"https://dev.azure.com/microsoft/{PROJECT}/_apis/build/builds/{BUILD_ID}"
    github = f"https://api.github.com/repos/{REPOSITORY}"
    github_token = os.environ.get("GH_TOKEN")
    validate_evidence(
        get_json(f"{ado}?api-version=7.1", token),
        get_json(f"{ado}/timeline?api-version=7.1", token),
        get_json(f"{ado}/artifacts?api-version=7.1", token),
        get_json(f"{github}/git/ref/tags/{TAG}", github_token),
        get_json(f"{github}/releases/tags/{TAG}", github_token),
        get_json(f"{github}/actions/runs/{PYTHON_RUN_ID}", github_token),
        get_json(f"{github}/actions/runs/{PYTHON_RUN_ID}/jobs?per_page=100", github_token),
    )


def verify_files(directory: Path, suffix: str) -> list[Path]:
    files = sorted(directory.rglob(f"*{suffix}"))
    names = [path.name for path in files]
    expected = {name for name in ASSETS if name.endswith(suffix)}
    require(
        len(names) == len(expected) and set(names) == expected,
        f"Package set mismatch for {suffix}: expected {sorted(expected)}, got {names}",
    )
    for path in files:
        digest = hashlib.sha256(path.read_bytes()).hexdigest()
        require(digest == ASSETS[path.name][1], f"Package SHA256 mismatch: {path.name}")
    return files


def verify_npm_metadata(path: Path) -> None:
    package = (
        "@microsoft/dynwinrt-codegen" if "-codegen-" in path.name else "@microsoft/dynwinrt"
    )
    with tarfile.open(path, "r:gz") as archive:
        members = [member for member in archive if member.name == "package/package.json"]
        require(len(members) == 1 and members[0].isfile(), f"Invalid npm manifest: {path.name}")
        with archive.extractfile(members[0]) as stream:
            manifest = json.load(stream)
    require(
        manifest["name"] == package and manifest["version"] == VERSION,
        f"npm name/version mismatch: {path.name}",
    )


def verify_packages(npm_directory: Path, python_directory: Path) -> list[Path]:
    npm = verify_files(npm_directory, ".tgz")
    verify_files(python_directory, ".whl")
    for path in npm:
        verify_npm_metadata(path)
    verify_release_set(argparse.Namespace(directory=python_directory, version=VERSION))
    return npm


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--npm-directory", required=True, type=Path)
    parser.add_argument("--python-directory", required=True, type=Path)
    parser.add_argument("--verified-npm-directory", required=True, type=Path)
    args = parser.parse_args()
    recovery_sha = validate_context(os.environ)
    validate_checkout(recovery_sha)
    validate_remote()
    npm = verify_packages(args.npm_directory, args.python_directory)
    destination = args.verified_npm_directory
    require(not destination.exists(), "Verified npm destination must be fresh")
    destination.mkdir(parents=True)
    for path in npm:
        shutil.copy2(path, destination / path.name)
    verify_files(destination, ".tgz")
    print(f"Recovery orchestration commit: {recovery_sha}")
    print(f"Package source (not recovery HEAD): {TAG} at {SOURCE_SHA}")
    print(f"Original npm run: {BUILD_ID}; tested Python run: {PYTHON_RUN_ID}")
    print("Provenance and package verification passed; no packages were built or published.")
    print(f"##vso[task.setvariable variable=version;isOutput=true;isReadOnly=true]{VERSION}")


if __name__ == "__main__":
    try:
        main()
    except (AssertionError, KeyError, ValueError, OSError, tarfile.TarError,
            subprocess.CalledProcessError) as error:
        print(f"preview.24 recovery validation failed: {error}", file=sys.stderr)
        raise SystemExit(1) from error
