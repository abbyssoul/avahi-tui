#!/usr/bin/env bash
#
# Run the libFuzzer targets via cargo-fuzz.
#
# Usage:
#   scripts/fuzz.sh [seconds-per-target] [target ...]
#
# Examples:
#   scripts/fuzz.sh                 # run every target for 60s each
#   scripts/fuzz.sh 300             # run every target for 5 minutes each
#   scripts/fuzz.sh 120 parse_command
#
# Environment:
#   FUZZ_SECONDS    default seconds per target (overridden by the first arg)
#   FUZZ_TOOLCHAIN  rustup toolchain to use (default: nightly)
#
# Requires a nightly toolchain; cargo-fuzz is installed automatically if missing.
set -euo pipefail

cd "$(dirname "$0")/.."

DURATION="${FUZZ_SECONDS:-60}"
if [[ "${1:-}" =~ ^[0-9]+$ ]]; then
    DURATION="$1"
    shift
fi

NIGHTLY="${FUZZ_TOOLCHAIN:-nightly}"

if ! cargo "+${NIGHTLY}" fuzz --version >/dev/null 2>&1; then
    echo "==> installing cargo-fuzz"
    cargo "+${NIGHTLY}" install cargo-fuzz --locked
fi

# Seed the parser corpus with the bundled command files so fuzzing starts from
# known-valid inputs instead of pure noise.
mkdir -p fuzz/corpus/parse_command
cp -f actions/*.toml fuzz/corpus/parse_command/ 2>/dev/null || true

targets=("$@")
if [[ ${#targets[@]} -eq 0 ]]; then
    mapfile -t targets < <(cargo "+${NIGHTLY}" fuzz list)
fi

status=0
for target in "${targets[@]}"; do
    echo "==> fuzzing '${target}' for ${DURATION}s"
    if ! cargo "+${NIGHTLY}" fuzz run "${target}" -- -max_total_time="${DURATION}"; then
        echo "!!! target '${target}' found a failure" >&2
        status=1
    fi
done

exit "${status}"
