#!/usr/bin/env python3
"""Classify the source-tree state recorded by the Omega RC bundler."""

from __future__ import annotations

import argparse
import hashlib
import subprocess
import sys
import tempfile
from pathlib import Path
from typing import Any


RELEASE_CHANNEL_PATH = Path("crates/omega/RELEASE_CHANNEL")


class SourceStateError(RuntimeError):
    pass


def run_git(repository: Path, *arguments: str) -> str:
    result = subprocess.run(
        ["git", "-C", str(repository), *arguments],
        capture_output=True,
        text=True,
        check=False,
    )
    if result.returncode != 0:
        detail = "\n".join(
            part.strip() for part in (result.stdout, result.stderr) if part.strip()
        )
        raise SourceStateError(
            f"git {' '.join(arguments)} failed" + (f": {detail}" if detail else "")
        )
    return result.stdout


def sha256_bytes(value: bytes) -> str:
    return hashlib.sha256(value).hexdigest()


def classify_worktree_state(
    repository: Path, expected_release_channel: str
) -> tuple[bool, list[str], list[dict[str, Any]]]:
    repository = repository.resolve()
    status = run_git(repository, "status", "--porcelain=v1", "--untracked-files=all")
    entries = [line for line in status.splitlines() if line.strip()]
    expected_path = RELEASE_CHANNEL_PATH.as_posix()
    expected_entry = f" M {expected_path}"
    expected_contents = f"{expected_release_channel}\n".encode()

    transformations: list[dict[str, Any]] = []
    unexpected_entries: list[str] = []
    for entry in entries:
        if entry != expected_entry:
            unexpected_entries.append(entry)
            continue
        channel_path = repository / RELEASE_CHANNEL_PATH
        try:
            observed_contents = channel_path.read_bytes()
            committed_contents = subprocess.run(
                ["git", "-C", str(repository), "show", f"HEAD:{expected_path}"],
                capture_output=True,
                check=True,
            ).stdout
        except (OSError, subprocess.CalledProcessError) as error:
            raise SourceStateError(
                "cannot validate the temporary release-channel transformation"
            ) from error
        if (
            observed_contents != expected_contents
            or committed_contents == observed_contents
        ):
            unexpected_entries.append(entry)
            continue
        transformations.append(
            {
                "kind": "temporary_release_channel",
                "path": expected_path,
                "committed_sha256": sha256_bytes(committed_contents),
                "generated_sha256": sha256_bytes(observed_contents),
                "generated_value": expected_release_channel,
            }
        )

    return bool(unexpected_entries), entries, transformations


def write(path: Path, contents: str) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(contents, encoding="utf-8")


def self_test() -> None:
    with tempfile.TemporaryDirectory(prefix="omega-rc-source-state-") as directory:
        repository = Path(directory)
        run_git(repository, "init", "--quiet")
        run_git(repository, "config", "user.email", "omega-source-state@example.invalid")
        run_git(repository, "config", "user.name", "Omega source state self-test")
        write(repository / RELEASE_CHANNEL_PATH, "dev\n")
        write(repository / "tracked.txt", "committed\n")
        run_git(repository, "add", RELEASE_CHANNEL_PATH.as_posix(), "tracked.txt")
        run_git(repository, "commit", "--quiet", "-m", "fixture")

        dirty, entries, transformations = classify_worktree_state(repository, "preview")
        if dirty or entries or transformations:
            raise SourceStateError("clean checkout was not classified as clean")

        write(repository / RELEASE_CHANNEL_PATH, "preview\n")
        dirty, entries, transformations = classify_worktree_state(repository, "preview")
        if dirty or entries != [f" M {RELEASE_CHANNEL_PATH.as_posix()}"]:
            raise SourceStateError("exact temporary channel rewrite was not accepted")
        if (
            len(transformations) != 1
            or transformations[0].get("generated_value") != "preview"
        ):
            raise SourceStateError("temporary channel rewrite was not recorded")

        write(repository / RELEASE_CHANNEL_PATH, "preview\nextra\n")
        dirty, _entries, transformations = classify_worktree_state(repository, "preview")
        if not dirty or transformations:
            raise SourceStateError("tampered channel rewrite was accepted")

        write(repository / RELEASE_CHANNEL_PATH, "preview\n")
        write(repository / "tracked.txt", "changed\n")
        dirty, _entries, _transformations = classify_worktree_state(repository, "preview")
        if not dirty:
            raise SourceStateError("unrelated tracked modification was accepted")

        write(repository / "tracked.txt", "committed\n")
        write(repository / "untracked.txt", "unexpected\n")
        dirty, _entries, _transformations = classify_worktree_state(repository, "preview")
        if not dirty:
            raise SourceStateError("unexpected untracked file was accepted")
        (repository / "untracked.txt").unlink()

        run_git(repository, "add", RELEASE_CHANNEL_PATH.as_posix())
        dirty, _entries, transformations = classify_worktree_state(repository, "preview")
        if not dirty or transformations:
            raise SourceStateError("staged channel modification was accepted")

    print("Omega RC source-state self-test passed")


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--self-test", action="store_true")
    arguments = parser.parse_args()
    if not arguments.self_test:
        parser.error("--self-test is required when invoking this helper directly")
    try:
        self_test()
    except SourceStateError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
