#!/usr/bin/env bash
# Smoke-run both bench suites: verifies all cases compile and execute.
# Fast exit — no statistics collected. Good for CI or pre-commit checks.
set -euo pipefail

cargo bench --bench shader_params -- --test
cargo bench --bench compositing   -- --test
