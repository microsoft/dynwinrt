# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""Conservative, PR-only scheduling of explanatory documentation changes."""

import json
import os
from pathlib import Path
import re
import subprocess


# Exact paths, not docs/** or *.md: packaged READMEs, generated capability
# reports, schemas, test data, and new/unknown documents still need full CI.
DOCUMENTATION = frozenset(
    """
README.md
docs/architecture/classic-com-contract-evidence-registry.md
docs/architecture/classic-com-generated-unsafe.md
docs/architecture/classic-com-raw-unsafe.md
docs/architecture/classic-com-support.md
docs/architecture/flat-win32-contracts.md
docs/architecture/javascript-binding-internals.md
docs/architecture/winrt-interface-implementations.md
docs/guides/development/ci.md
docs/guides/node/dev-mode.md
docs/guides/python/python-ui-ecosystem.md
docs/guides/windows/classic-com-usage.md
docs/guides/windows/msix-packaging.md
docs/guides/windows/winrt-interface-implementations.md
samples/js/README.md
samples/js/electron-aion-chat/README.md
samples/js/electron-share-ui/README.md
samples/js/electron-smtc/README.md
samples/js/interface-implementation/README.md
samples/js/low-level/README.md
samples/js/ocr/README.md
samples/js/win32/README.md
samples/js/windows-hello/README.md
samples/js/winui-tic-tac-toe-code-only/README.md
samples/js/winui-tic-tac-toe/README.md
samples/python/README.md
samples/python/app-lifecycle-single-instance/README.md
samples/python/app-notification/README.md
samples/python/async-file-io/README.md
samples/python/cryptography/README.md
samples/python/custom-winmd-codegen/README.md
samples/python/device-watcher/README.md
samples/python/interface-implementation/README.md
samples/python/ocr-image/README.md
samples/python/text-to-speech/README.md
samples/python/winui-hello-world/README.md
samples/python/winui-tic-tac-toe-code-only/README.md
samples/python/winui-tic-tac-toe/README.md
""".split()
)


def documentation_diff(diff: bytes) -> bool:
    if not diff:
        return False
    fields = diff.decode("utf-8").split("\0")
    if fields.pop() != "":
        raise ValueError("Git name-status output was not NUL-terminated")
    index = 0
    while index < len(fields):
        status = fields[index]
        count = 2 if re.fullmatch(r"R\d+", status) else 1
        paths = fields[index + 1 : index + 1 + count]
        if len(paths) != count:
            raise ValueError("Incomplete Git name-status record")
        if status not in {"A", "M", "D"} and count != 2:
            return False
        if any(path not in DOCUMENTATION for path in paths):
            return False
        index += 1 + count
    return True


def classify(event_name: str, event: dict, repo: Path) -> bool:
    if event_name != "pull_request":
        return False
    pull_request = event["pull_request"]
    base, head = (pull_request[side]["sha"] for side in ("base", "head"))
    for sha in (base, head):
        if not isinstance(sha, str) or not re.fullmatch(r"[0-9a-f]{40}", sha):
            raise ValueError("PR base/head must be full Git commit IDs")
    merge_base = subprocess.check_output(
        ["git", "merge-base", base, head], cwd=repo
    ).decode("ascii").strip()
    if not re.fullmatch(r"[0-9a-f]{40}", merge_base):
        raise ValueError("Git did not resolve a unique PR merge base")
    diff = subprocess.check_output(
        ["git", "diff", "--name-status", "--find-renames", "-z", merge_base, head, "--"],
        cwd=repo,
    )
    return documentation_diff(diff)


def main() -> None:
    event_name = os.environ["GITHUB_EVENT_NAME"]
    event = json.loads(Path(os.environ["GITHUB_EVENT_PATH"]).read_text(encoding="utf-8"))
    docs_only = classify(event_name, event, Path.cwd())
    print(f"CI mode: {'documentation-only PR' if docs_only else 'full validation'}")
    with open(os.environ["GITHUB_OUTPUT"], "a", encoding="utf-8") as output:
        output.write(f"docs_only={str(docs_only).lower()}\n")


if __name__ == "__main__":
    main()
