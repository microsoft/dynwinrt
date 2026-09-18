# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

"""PR-only lightweight scheduling for Markdown changes."""

import json
import os
from pathlib import Path
import re
import subprocess


def documentation_diff(diff: bytes) -> bool:
    if not diff:
        return False
    fields = diff.decode("utf-8").split("\0")
    if fields.pop() != "":
        raise ValueError("Git name-status output was not NUL-terminated")
    index = 0
    while index < len(fields):
        status = fields[index]
        rename = re.fullmatch(r"R[0-9]{1,3}", status) is not None and int(status[1:]) <= 100
        count = 2 if rename else 1
        paths = fields[index + 1 : index + 1 + count]
        if len(paths) != count:
            raise ValueError("Incomplete Git name-status record")
        if status not in {"A", "M", "D"} and not rename:
            return False
        if any(not path.lower().endswith(".md") for path in paths):
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
