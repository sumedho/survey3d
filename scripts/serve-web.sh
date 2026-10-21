#!/usr/bin/env bash
# Serves the web build with hot reload at http://127.0.0.1:8765
#
# SQLite is compiled to wasm with clang, which needs an LLVM that has the wasm32
# target. Apple's Xcode clang does not, so on macOS install one with
# `brew install llvm`. Linux distributions' clang usually works as-is.
set -euo pipefail
cd "$(dirname "$0")/../crates/survey-app"

if [[ "$(uname)" == "Darwin" && -z "${CC_wasm32_unknown_unknown:-}" ]]; then
    llvm="$(brew --prefix llvm 2>/dev/null || true)"
    if [[ -x "$llvm/bin/clang" ]]; then
        export CC_wasm32_unknown_unknown="$llvm/bin/clang"
        export AR_wasm32_unknown_unknown="$llvm/bin/llvm-ar"
    else
        echo "warning: Homebrew LLVM not found; the wasm SQLite build may fail (brew install llvm)" >&2
    fi
fi

exec trunk serve "$@"
