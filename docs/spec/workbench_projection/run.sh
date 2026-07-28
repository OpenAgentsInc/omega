#!/bin/sh

set -eu

script_directory=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd)
tlc_binary=${TLC:-tlc}
tla2tools_jar=${TLA2TOOLS_JAR:-}
java_binary=${JAVA:-java}
worker_count=${TLC_WORKERS:-4}
scratch_directory=$(mktemp -d "${TMPDIR:-/tmp}/omega-workbench-tlc.XXXXXX")

cleanup() {
    find "$scratch_directory" -depth -delete
}
trap cleanup EXIT HUP INT TERM

if [ -n "$tla2tools_jar" ]; then
    if [ ! -f "$tla2tools_jar" ]; then
        echo "tla2tools.jar not found: $tla2tools_jar" >&2
        exit 127
    fi
    if ! command -v "$java_binary" >/dev/null 2>&1; then
        echo "Java executable not found: $java_binary" >&2
        exit 127
    fi
elif ! command -v "$tlc_binary" >/dev/null 2>&1; then
    echo "TLC executable not found: $tlc_binary" >&2
    echo "Set TLC to a TLC executable or TLA2TOOLS_JAR to tla2tools.jar." >&2
    exit 127
fi

invoke_tlc() {
    if [ -n "$tla2tools_jar" ]; then
        "$java_binary" -cp "$tla2tools_jar" tlc2.TLC "$@"
    else
        "$tlc_binary" "$@"
    fi
}

run_tlc() {
    label=$1
    configuration=$2
    output_file="$scratch_directory/$label.out"
    metadir="$scratch_directory/$label-states"

    if invoke_tlc \
        -workers "$worker_count" \
        -cleanup \
        -metadir "$metadir" \
        -config "$configuration" \
        "$script_directory/WorkbenchProjection.tla" \
        >"$output_file" 2>&1
    then
        return 0
    else
        return $?
    fi
}

print_statistics() {
    output_file=$1
    grep -E -m1 '[0-9]+ states generated, [0-9]+ distinct states found' \
        "$output_file" || true
}

expect_pass() {
    label=$1
    configuration=$2

    if ! run_tlc "$label" "$configuration"; then
        echo "FAIL $label: expected TLC to pass" >&2
        tail -n 40 "$scratch_directory/$label.out" >&2
        exit 1
    fi

    echo "PASS $label: $(print_statistics "$scratch_directory/$label.out")"
}

expect_counterexample() {
    label=$1
    configuration=$2
    expected_message=$3

    if run_tlc "$label" "$configuration"; then
        echo "FAIL $label: expected a TLC counterexample" >&2
        tail -n 40 "$scratch_directory/$label.out" >&2
        exit 1
    fi

    if ! grep -F "$expected_message" "$scratch_directory/$label.out" >/dev/null; then
        echo "FAIL $label: TLC failed for an unexpected reason" >&2
        tail -n 40 "$scratch_directory/$label.out" >&2
        exit 1
    fi

    echo "PASS $label: expected counterexample found"
}

expect_pass "base" "$script_directory/WorkbenchProjection.cfg"

expect_counterexample \
    "probe-cold-restore" \
    "$script_directory/reachability/ColdRestore.cfg" \
    "Invariant Probe_ColdRestore_Unreached is violated"
expect_counterexample \
    "probe-reconnect" \
    "$script_directory/reachability/Reconnect.cfg" \
    "Invariant Probe_Reconnect_Unreached is violated"
expect_counterexample \
    "probe-invalid-fallback" \
    "$script_directory/reachability/InvalidFallback.cfg" \
    "Invariant Probe_InvalidFallback_Unreached is violated"
expect_counterexample \
    "probe-stale-completion" \
    "$script_directory/reachability/StaleCompletion.cfg" \
    "Invariant Probe_StaleCompletion_Unreached is violated"
expect_counterexample \
    "probe-hidden-current-completion" \
    "$script_directory/reachability/HiddenCurrentCompletion.cfg" \
    "Invariant Probe_HiddenCurrentCompletion_Unreached is violated"
expect_pass \
    "probe-stale-completion-disabled" \
    "$script_directory/reachability/StaleCompletionDisabled.cfg"

expect_counterexample \
    "mutation-binding-accepts-stale" \
    "$script_directory/mutations/BindingAcceptsStale.cfg" \
    "Invariant Inv_BindingSafety is violated"
expect_counterexample \
    "mutation-hidden-surface-owns-focus" \
    "$script_directory/mutations/HiddenSurfaceOwnsFocus.cfg" \
    "Invariant Inv_SingleOwner is violated"
expect_counterexample \
    "mutation-missing-worktree-kept" \
    "$script_directory/mutations/MissingWorktreeKept.cfg" \
    "Invariant Inv_SelectionValidity is violated"
expect_counterexample \
    "mutation-older-snapshot-applied" \
    "$script_directory/mutations/OlderSnapshotApplied.cfg" \
    "Invariant Inv_PersistenceMonotonicity is violated"
expect_counterexample \
    "mutation-previous-thread-isolation" \
    "$script_directory/mutations/PreviousThreadIsolation.cfg" \
    "Invariant Inv_ThreadIsolation is violated"
expect_counterexample \
    "mutation-previous-thread-restore" \
    "$script_directory/mutations/PreviousThreadRestore.cfg" \
    "Invariant Inv_RestoreFidelity is violated"
expect_counterexample \
    "mutation-settle-disabled" \
    "$script_directory/mutations/SettleDisabled.cfg" \
    "Temporal properties were violated"
expect_counterexample \
    "mutation-stale-completion-accepted" \
    "$script_directory/mutations/StaleCompletionAccepted.cfg" \
    "Invariant Inv_StaleCompletionImmunity is violated"
expect_counterexample \
    "mutation-unordered-fallback" \
    "$script_directory/mutations/UnorderedFallback.cfg" \
    "Invariant Inv_SelectionValidity is violated"

echo "All workbench projection TLC checks passed."
