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
INSTALLED_OBSERVATIONS_SCHEMA = "openagents.omega.identity-installed-observations.v1"
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
    "offline-create-and-encrypted-recovery-protection",
    "wrong-recovery-password-rejection",
    "corrupt-recovery-artifact-rejection",
    "encrypted-recovery-and-restart-continuity",
}
TRIPWIRE_SURFACES = {
    "logs",
    "telemetry",
    "clipboard",
    "accessibility",
    "diagnostics",
    "crashes",
}
MANUAL_JOURNEY_CHECKS = {
    "identity-first-first-run",
    "theme-and-agent-setup-baseline",
    "zed-data-before-after-isolation",
}
ACCESSIBILITY_CHECKS = {
    "keyboard-focus-traversal",
    "screen-reader-output",
    "viewport-360-pixels",
    "larger-ui-font",
    "light-theme",
    "dark-theme",
    "high-contrast",
    "reduced-motion",
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


def validate_installed_observations(path: Path, candidate_digest: str, evidence_root: Path) -> str:
    if path.is_symlink() or not path.is_file():
        raise IdentityEvidenceError(
            "installed identity observations do not name a regular file"
        )
    report = load_json(path, "installed identity observations")
    if set(report) != {
        "schema",
        "candidate_digest",
        "status",
        "manual_journey",
        "accessibility",
        "evidence_sha256",
    }:
        raise IdentityEvidenceError("installed identity observation keys are not exact")
    if (
        report.get("schema") != INSTALLED_OBSERVATIONS_SCHEMA
        or report.get("candidate_digest") != candidate_digest
        or report.get("status") != "passed"
    ):
        raise IdentityEvidenceError(
            "installed identity observations are not passed or candidate-bound"
        )
    validate_observation_group(
        report.get("manual_journey"),
        MANUAL_JOURNEY_CHECKS,
        evidence_root,
    )
    validate_observation_group(
        report.get("accessibility"),
        ACCESSIBILITY_CHECKS,
        evidence_root,
    )
    claimed = require_sha256(
        report.get("evidence_sha256"), "installed observation evidence digest"
    )
    canonical = dict(report)
    canonical.pop("evidence_sha256")
    if canonical_digest(canonical) != claimed:
        raise IdentityEvidenceError("installed identity observation digest differs")
    return sha256_file(path)


def validate_observation_group(
    observations: Any,
    expected_checks: set[str],
    evidence_root: Path,
) -> None:
    if not isinstance(observations, list) or len(observations) != len(expected_checks):
        raise IdentityEvidenceError("installed observation check inventory is incomplete")
    observed: set[str] = set()
    for observation in observations:
        if not isinstance(observation, dict) or set(observation) != {
            "check",
            "status",
            "observed_at",
            "facts",
            "evidence_refs",
        }:
            raise IdentityEvidenceError("installed observation entry is not exact")
        check = observation.get("check")
        if check in observed or check not in expected_checks:
            raise IdentityEvidenceError("installed observation check is invalid")
        if observation.get("status") != "passed":
            raise IdentityEvidenceError(f"installed observation {check} is not passed")
        require_timestamp(observation.get("observed_at"), f"installed observation {check}")
        validate_observation_facts(check, observation.get("facts"))
        references = observation.get("evidence_refs")
        if not isinstance(references, list) or not references:
            raise IdentityEvidenceError(
                f"installed observation {check} has no evidence references"
            )
        resolved = [resolve_evidence_reference(reference, evidence_root) for reference in references]
        if len({(path, digest) for path, digest in resolved}) != len(resolved):
            raise IdentityEvidenceError(
                f"installed observation {check} repeats an evidence reference"
            )
        observed.add(check)
    if observed != expected_checks:
        raise IdentityEvidenceError("installed observation check inventory differs")


def validate_observation_facts(check: str, facts: Any) -> None:
    if not isinstance(facts, dict):
        raise IdentityEvidenceError(f"installed observation {check} facts are invalid")
    exact_facts: dict[str, dict[str, Any]] = {
        "identity-first-first-run": {
            "identity_presented_before_editor_setup": True,
            "identity_ready": True,
        },
        "theme-and-agent-setup-baseline": {
            "theme_families": ["Aiur", "Ayu", "Gruvbox"],
            "agent_setup_visible": True,
        },
        "keyboard-focus-traversal": {
            "all_controls_reachable": True,
            "reverse_traversal": True,
            "focus_visible": True,
            "keyboard_activation": True,
        },
        "viewport-360-pixels": {
            "viewport_width_pixels": 360,
            "horizontal_overflow": False,
            "completion_action_visible": True,
        },
        "light-theme": {"appearance": "light", "content_legible": True},
        "dark-theme": {"appearance": "dark", "content_legible": True},
        "high-contrast": {
            "system_increase_contrast": True,
            "content_legible": True,
            "focus_indicator_visible": True,
        },
        "reduced-motion": {
            "system_reduce_motion": True,
            "motion_required_for_completion": False,
        },
    }
    if check in exact_facts:
        if facts != exact_facts[check]:
            raise IdentityEvidenceError(f"installed observation {check} facts differ")
        return
    if check == "zed-data-before-after-isolation":
        if set(facts) != {"zed_before_sha256", "zed_after_sha256", "unchanged"}:
            raise IdentityEvidenceError("Zed isolation facts are not exact")
        before = require_sha256(facts.get("zed_before_sha256"), "Zed before digest")
        after = require_sha256(facts.get("zed_after_sha256"), "Zed after digest")
        if before != after or facts.get("unchanged") is not True:
            raise IdentityEvidenceError("Zed isolation observation changed")
        return
    if check == "screen-reader-output":
        if set(facts) != {
            "assistive_technology",
            "identity_status_announced",
            "controls_named",
            "secret_value_exposed",
        }:
            raise IdentityEvidenceError("screen-reader facts are not exact")
        technology = facts.get("assistive_technology")
        if (
            not isinstance(technology, str)
            or not technology.strip()
            or facts.get("identity_status_announced") is not True
            or facts.get("controls_named") is not True
            or facts.get("secret_value_exposed") is not False
        ):
            raise IdentityEvidenceError("screen-reader observation did not pass safely")
        return
    if check == "larger-ui-font":
        if set(facts) != {
            "ui_font_size_pixels",
            "content_clipped",
            "completion_action_visible",
        }:
            raise IdentityEvidenceError("larger UI font facts are not exact")
        font_size = facts.get("ui_font_size_pixels")
        if (
            not isinstance(font_size, int)
            or isinstance(font_size, bool)
            or font_size < 18
            or facts.get("content_clipped") is not False
            or facts.get("completion_action_visible") is not True
        ):
            raise IdentityEvidenceError("larger UI font observation did not pass")
        return
    raise IdentityEvidenceError(f"unknown installed observation check {check}")


def require_timestamp(value: Any, label: str) -> None:
    try:
        parsed = datetime.fromisoformat(value.replace("Z", "+00:00"))
    except (AttributeError, ValueError) as error:
        raise IdentityEvidenceError(f"{label} timestamp is invalid") from error
    if parsed.tzinfo is None:
        raise IdentityEvidenceError(f"{label} timestamp lacks a timezone")


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
    if not evidence_root.is_dir() or evidence_root.is_symlink():
        raise IdentityEvidenceError("evidence root is missing or unsafe")
    path = evidence_root / relative
    current = evidence_root
    for component in Path(relative).parts:
        current = current / component
        if current.is_symlink():
            raise IdentityEvidenceError("evidence reference contains a symbolic link")
    if not path.is_file():
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
    observation_facts = {
        "identity-first-first-run": {
            "identity_presented_before_editor_setup": True,
            "identity_ready": True,
        },
        "theme-and-agent-setup-baseline": {
            "theme_families": ["Aiur", "Ayu", "Gruvbox"],
            "agent_setup_visible": True,
        },
        "zed-data-before-after-isolation": {
            "zed_before_sha256": "2" * 64,
            "zed_after_sha256": "2" * 64,
            "unchanged": True,
        },
        "keyboard-focus-traversal": {
            "all_controls_reachable": True,
            "reverse_traversal": True,
            "focus_visible": True,
            "keyboard_activation": True,
        },
        "screen-reader-output": {
            "assistive_technology": "self-test reader",
            "identity_status_announced": True,
            "controls_named": True,
            "secret_value_exposed": False,
        },
        "viewport-360-pixels": {
            "viewport_width_pixels": 360,
            "horizontal_overflow": False,
            "completion_action_visible": True,
        },
        "larger-ui-font": {
            "ui_font_size_pixels": 18,
            "content_clipped": False,
            "completion_action_visible": True,
        },
        "light-theme": {"appearance": "light", "content_legible": True},
        "dark-theme": {"appearance": "dark", "content_legible": True},
        "high-contrast": {
            "system_increase_contrast": True,
            "content_legible": True,
            "focus_indicator_visible": True,
        },
        "reduced-motion": {
            "system_reduce_motion": True,
            "motion_required_for_completion": False,
        },
    }
    with tempfile.TemporaryDirectory(prefix="omega-identity-evidence-") as directory:
        root = Path(directory)
        matrix_path = root / "matrix.json"
        tripwire_path = root / "tripwires.json"
        observation_evidence_path = root / "installed-observation.txt"
        observation_evidence_path.write_text("installed observation fixture", encoding="utf-8")
        observation_reference = {
            "path": observation_evidence_path.name,
            "sha256": sha256_file(observation_evidence_path),
        }
        observations = {
            "schema": INSTALLED_OBSERVATIONS_SCHEMA,
            "candidate_digest": candidate_digest,
            "status": "passed",
            "manual_journey": [
                {
                    "check": check,
                    "status": "passed",
                    "observed_at": "2026-07-24T12:00:00+00:00",
                    "facts": observation_facts[check],
                    "evidence_refs": [observation_reference],
                }
                for check in sorted(MANUAL_JOURNEY_CHECKS)
            ],
            "accessibility": [
                {
                    "check": check,
                    "status": "passed",
                    "observed_at": "2026-07-24T12:00:00+00:00",
                    "facts": observation_facts[check],
                    "evidence_refs": [observation_reference],
                }
                for check in sorted(ACCESSIBILITY_CHECKS)
            ],
        }
        observations["evidence_sha256"] = canonical_digest(observations)
        observations_path = root / "installed-observations.json"
        matrix_path.write_text(json.dumps(matrix), encoding="utf-8")
        tripwire_path.write_text(json.dumps(tripwires), encoding="utf-8")
        observations_path.write_text(json.dumps(observations), encoding="utf-8")
        matrix_digest = validate_identity_matrix(matrix_path, candidate_digest)
        tripwire_digest = validate_installed_tripwires(tripwire_path, candidate_digest)
        observation_digest = validate_installed_observations(
            observations_path, candidate_digest, root
        )
        resolve_evidence_reference(
            {"path": matrix_path.name, "sha256": matrix_digest}, root
        )
        resolve_evidence_reference(
            {"path": tripwire_path.name, "sha256": tripwire_digest}, root
        )
        resolve_evidence_reference(
            {"path": observations_path.name, "sha256": observation_digest}, root
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
        observations["accessibility"][0]["facts"] = {}
        observations_path.write_text(json.dumps(observations), encoding="utf-8")
        try:
            validate_installed_observations(observations_path, candidate_digest, root)
        except IdentityEvidenceError:
            pass
        else:
            raise IdentityEvidenceError("invalid accessibility facts were accepted")
        valid_observations = json.loads(
            json.dumps(observations)
        )
        valid_observations["accessibility"][0]["facts"] = observation_facts[
            valid_observations["accessibility"][0]["check"]
        ]
        invalid_variants = []
        missing = json.loads(json.dumps(valid_observations))
        missing["accessibility"].pop()
        missing["evidence_sha256"] = canonical_digest(
            {key: value for key, value in missing.items() if key != "evidence_sha256"}
        )
        invalid_variants.append(missing)
        duplicate = json.loads(json.dumps(valid_observations))
        duplicate["accessibility"][1]["check"] = duplicate["accessibility"][0]["check"]
        duplicate["evidence_sha256"] = canonical_digest(
            {key: value for key, value in duplicate.items() if key != "evidence_sha256"}
        )
        invalid_variants.append(duplicate)
        stale = json.loads(json.dumps(valid_observations))
        stale["candidate_digest"] = "9" * 64
        stale["evidence_sha256"] = canonical_digest(
            {key: value for key, value in stale.items() if key != "evidence_sha256"}
        )
        invalid_variants.append(stale)
        forged = json.loads(json.dumps(valid_observations))
        forged["manual_journey"][0]["evidence_refs"][0]["sha256"] = "8" * 64
        forged["evidence_sha256"] = canonical_digest(
            {key: value for key, value in forged.items() if key != "evidence_sha256"}
        )
        invalid_variants.append(forged)
        for invalid in invalid_variants:
            observations_path.write_text(json.dumps(invalid), encoding="utf-8")
            try:
                validate_installed_observations(observations_path, candidate_digest, root)
            except IdentityEvidenceError:
                pass
            else:
                raise IdentityEvidenceError("invalid installed observations were accepted")
    print("Omega identity evidence receipt self-test passed")


if __name__ == "__main__":
    self_test()
