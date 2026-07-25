#!/usr/bin/env python3

from __future__ import annotations

import hashlib
import json
import re
import tempfile
from datetime import datetime
from pathlib import Path
from typing import Any


MATRIX_SCHEMA = "openagents.omega.identity-proof-matrix.v1"
TRIPWIRE_SCHEMA = "openagents.omega.installed-secret-tripwires.v1"
MATRIX_CASES = {
    "disposable-namespace-safety",
    "create-read-back-restart-sign",
    "double-create",
    "concurrent-create-and-process-start",
    "forged-request-rejection",
    "stale-request-rejection",
    "crash-after-secret-write",
    "crash-after-secret-read-back",
    "crash-after-manifest-commit",
    "crash-after-reset-marker",
    "crash-after-reset-commit",
    "crash-after-relaunch-acknowledge",
    "simulated-conflict-custody",
    "simulated-lost-custody",
    "simulated-locked-custody",
    "simulated-symlink-refusal",
    "simulated-weak-permission-refusal",
    "simulated-keychain-unavailable",
    "simulated-corrupt-keychain",
    "simulated-malformed-event-rejection",
    "simulated-unadmitted-purpose-rejection",
    "simulated-conflicting-recovery-selection",
    "simulated-late-completion-fencing",
    "simulated-signer-crash-before-completion",
}
TRIPWIRE_SURFACES = {
    "logs",
    "telemetry",
    "clipboard",
    "accessibility",
    "diagnostics",
    "crashes",
}
SHA256 = re.compile(r"^[0-9a-f]{64}$")


class IdentityEvidenceError(RuntimeError):
    pass


def reject_duplicate_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    value: dict[str, Any] = {}
    for key, item in pairs:
        if key in value:
            raise IdentityEvidenceError(f"duplicate JSON key: {key}")
        value[key] = item
    return value


def load_json(path: Path, label: str) -> dict[str, Any]:
    try:
        value = json.loads(
            path.read_text(encoding="utf-8"), object_pairs_hook=reject_duplicate_keys
        )
    except (OSError, UnicodeError, json.JSONDecodeError) as error:
        raise IdentityEvidenceError(f"cannot read {label}") from error
    if not isinstance(value, dict):
        raise IdentityEvidenceError(f"{label} must be a JSON object")
    return value


def sha256_file(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def canonical_digest(value: Any) -> str:
    encoded = json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=False
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()


def require_sha256(value: Any, label: str) -> str:
    if not isinstance(value, str) or SHA256.fullmatch(value) is None:
        raise IdentityEvidenceError(f"{label} must be a lowercase SHA-256 digest")
    return value


def validate_identity_matrix(path: Path, candidate_digest: str) -> str:
    report = load_json(path, "identity proof matrix")
    if set(report) != {
        "schema",
        "status",
        "candidate",
        "disposable_keychain",
        "cases",
        "evidence_sha256",
    }:
        raise IdentityEvidenceError("identity proof matrix keys are not exact")
    if report.get("schema") != MATRIX_SCHEMA or report.get("status") != "passed":
        raise IdentityEvidenceError("identity proof matrix is not passed")
    candidate = report.get("candidate")
    if not isinstance(candidate, dict) or set(candidate) != {
        "candidate_digest",
        "artifact_sha256",
        "release_record_sha256",
        "identity_proof_binary_sha256",
    }:
        raise IdentityEvidenceError("identity proof matrix candidate binding is not exact")
    if candidate.get("candidate_digest") != candidate_digest:
        raise IdentityEvidenceError("identity proof matrix binds a different candidate")
    for name, value in candidate.items():
        require_sha256(value, f"identity proof matrix candidate.{name}")
    locator = report.get("disposable_keychain")
    if locator != {
        "service": "com.openagents.omega.identity-proof.v1",
        "account": "disposable-proof-only",
        "cleanup": "passed",
        "production_locator_access": "rejected-by-construction",
    }:
        raise IdentityEvidenceError("identity proof matrix locator or cleanup is unsafe")
    cases = report.get("cases")
    if not isinstance(cases, list) or len(cases) != len(MATRIX_CASES):
        raise IdentityEvidenceError("identity proof matrix case inventory is incomplete")
    observed: set[str] = set()
    for case in cases:
        if not isinstance(case, dict) or set(case) != {
            "case",
            "status",
            "evidence_sha256",
        }:
            raise IdentityEvidenceError("identity proof matrix case is not exact")
        name = case.get("case")
        if name in observed or name not in MATRIX_CASES or case.get("status") != "passed":
            raise IdentityEvidenceError("identity proof matrix case is invalid")
        require_sha256(case.get("evidence_sha256"), f"identity proof matrix case {name}")
        observed.add(name)
    if observed != MATRIX_CASES:
        raise IdentityEvidenceError("identity proof matrix case inventory differs")
    claimed = require_sha256(report.get("evidence_sha256"), "matrix evidence digest")
    canonical = dict(report)
    canonical.pop("evidence_sha256")
    if canonical_digest(canonical) != claimed:
        raise IdentityEvidenceError("identity proof matrix internal digest differs")
    return sha256_file(path)


def validate_installed_tripwires(path: Path, candidate_digest: str) -> str:
    report = load_json(path, "installed secret tripwires")
    if set(report) != {
        "schema",
        "candidate_digest",
        "generated_at",
        "status",
        "needle",
        "surfaces",
    }:
        raise IdentityEvidenceError("installed tripwire receipt keys are not exact")
    if (
        report.get("schema") != TRIPWIRE_SCHEMA
        or report.get("candidate_digest") != candidate_digest
        or report.get("status") != "pass"
    ):
        raise IdentityEvidenceError("installed tripwire receipt is not passed or candidate-bound")
    generated_at = report.get("generated_at")
    try:
        parsed_generated_at = datetime.fromisoformat(
            generated_at.replace("Z", "+00:00")
        )
    except (AttributeError, ValueError) as error:
        raise IdentityEvidenceError("installed tripwire timestamp is invalid") from error
    if parsed_generated_at.tzinfo is None:
        raise IdentityEvidenceError("installed tripwire timestamp lacks a timezone")
    if report.get("needle") != {
        "source": "protected_file_descriptor",
        "length": report.get("needle", {}).get("length"),
        "value_recorded": False,
    }:
        raise IdentityEvidenceError("installed tripwire needle handling is unsafe")
    length = report["needle"]["length"]
    if not isinstance(length, int) or not 16 <= length <= 4096:
        raise IdentityEvidenceError("installed tripwire needle length is invalid")
    surfaces = report.get("surfaces")
    if not isinstance(surfaces, list) or len(surfaces) != len(TRIPWIRE_SURFACES):
        raise IdentityEvidenceError("installed tripwire surface inventory is incomplete")
    observed: set[str] = set()
    expected_keys = {
        "name",
        "path_digest",
        "status",
        "files_scanned",
        "symlinks_skipped",
        "bytes_scanned",
        "errors",
        "match_detected",
        "evidence_digest",
    }
    for surface in surfaces:
        if not isinstance(surface, dict) or set(surface) != expected_keys:
            raise IdentityEvidenceError("installed tripwire surface is not exact")
        name = surface.get("name")
        if name in observed or name not in TRIPWIRE_SURFACES:
            raise IdentityEvidenceError("installed tripwire surface name is invalid")
        if (
            surface.get("status") not in ("pass", "absent")
            or surface.get("errors") != 0
            or surface.get("match_detected") is not False
        ):
            raise IdentityEvidenceError("installed tripwire surface did not pass safely")
        for field in (
            "files_scanned",
            "symlinks_skipped",
            "bytes_scanned",
            "errors",
        ):
            value = surface.get(field)
            if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                raise IdentityEvidenceError("installed tripwire surface counts are invalid")
        require_sha256(surface.get("path_digest"), f"tripwire {name} path digest")
        require_sha256(surface.get("evidence_digest"), f"tripwire {name} evidence digest")
        observed.add(name)
    if observed != TRIPWIRE_SURFACES:
        raise IdentityEvidenceError("installed tripwire surface inventory differs")
    return sha256_file(path)


def resolve_evidence_reference(reference: Any, evidence_root: Path) -> tuple[Path, str]:
    if not isinstance(reference, dict) or set(reference) != {"path", "sha256"}:
        raise IdentityEvidenceError("evidence reference must contain exact path and sha256")
    relative = reference.get("path")
    expected = require_sha256(reference.get("sha256"), "evidence reference sha256")
    if (
        not isinstance(relative, str)
        or not relative
        or Path(relative).is_absolute()
        or ".." in Path(relative).parts
    ):
        raise IdentityEvidenceError("evidence reference path is unsafe")
    path = evidence_root / relative
    if path.is_symlink() or not path.is_file():
        raise IdentityEvidenceError("evidence reference does not name a regular file")
    if sha256_file(path) != expected:
        raise IdentityEvidenceError("evidence reference digest differs")
    return path, expected


def self_test() -> None:
    candidate_digest = "a" * 64
    candidate = {
        "candidate_digest": candidate_digest,
        "artifact_sha256": "b" * 64,
        "release_record_sha256": "c" * 64,
        "identity_proof_binary_sha256": "d" * 64,
    }
    cases = [
        {"case": name, "status": "passed", "evidence_sha256": "e" * 64}
        for name in sorted(MATRIX_CASES)
    ]
    matrix = {
        "schema": MATRIX_SCHEMA,
        "status": "passed",
        "candidate": candidate,
        "disposable_keychain": {
            "service": "com.openagents.omega.identity-proof.v1",
            "account": "disposable-proof-only",
            "cleanup": "passed",
            "production_locator_access": "rejected-by-construction",
        },
        "cases": cases,
    }
    matrix["evidence_sha256"] = canonical_digest(matrix)
    surfaces = [
        {
            "name": name,
            "path_digest": "f" * 64,
            "status": "pass",
            "files_scanned": 1,
            "symlinks_skipped": 0,
            "bytes_scanned": 1,
            "errors": 0,
            "match_detected": False,
            "evidence_digest": "1" * 64,
        }
        for name in sorted(TRIPWIRE_SURFACES)
    ]
    tripwires = {
        "schema": TRIPWIRE_SCHEMA,
        "candidate_digest": candidate_digest,
        "generated_at": "2026-07-24T12:00:00+00:00",
        "status": "pass",
        "needle": {
            "source": "protected_file_descriptor",
            "length": 32,
            "value_recorded": False,
        },
        "surfaces": surfaces,
    }
    with tempfile.TemporaryDirectory(prefix="omega-identity-evidence-") as directory:
        root = Path(directory)
        matrix_path = root / "matrix.json"
        tripwire_path = root / "tripwires.json"
        matrix_path.write_text(json.dumps(matrix), encoding="utf-8")
        tripwire_path.write_text(json.dumps(tripwires), encoding="utf-8")
        matrix_digest = validate_identity_matrix(matrix_path, candidate_digest)
        tripwire_digest = validate_installed_tripwires(tripwire_path, candidate_digest)
        resolve_evidence_reference(
            {"path": matrix_path.name, "sha256": matrix_digest}, root
        )
        resolve_evidence_reference(
            {"path": tripwire_path.name, "sha256": tripwire_digest}, root
        )
        matrix["cases"].pop()
        matrix_path.write_text(json.dumps(matrix), encoding="utf-8")
        try:
            validate_identity_matrix(matrix_path, candidate_digest)
        except IdentityEvidenceError:
            pass
        else:
            raise IdentityEvidenceError("truncated matrix was accepted")
        tripwires["surfaces"][0]["match_detected"] = True
        tripwire_path.write_text(json.dumps(tripwires), encoding="utf-8")
        try:
            validate_installed_tripwires(tripwire_path, candidate_digest)
        except IdentityEvidenceError:
            pass
        else:
            raise IdentityEvidenceError("matching tripwire receipt was accepted")
    print("Omega identity evidence receipt self-test passed")


if __name__ == "__main__":
    self_test()
