#!/usr/bin/env bash
# Run the pinned C formatter for the kernel-side C sources (kmod + builtin).
set -euo pipefail

cd "$(dirname "$0")/.."

usage() {
    cat >&2 <<'EOF'
Usage: scripts/clang-format-c.sh [--check|--fix] [file...]

Uses clang-format 18.x. Set CLANG_FORMAT=/path/to/clang-format to override.
When no files are passed, formats all tracked kmod and builtin C/H files except
generated and vendored sources. builtin/.clang-format is a symlink to
kmod/.clang-format so both trees share one style.
EOF
}

mode="check"
case "${1:-}" in
    --check)
        mode="check"
        shift
        ;;
    --fix)
        mode="fix"
        shift
        ;;
    -h|--help)
        usage
        exit 0
        ;;
esac

find_clang_format() {
    if [[ -n "${CLANG_FORMAT:-}" ]]; then
        printf '%s\n' "$CLANG_FORMAT"
        return
    fi

    local candidates=(
        clang-format-18
        /usr/lib/llvm18/bin/clang-format
        /usr/bin/clang-format-18
        clang-format
    )
    local candidate
    for candidate in "${candidates[@]}"; do
        if command -v "$candidate" >/dev/null 2>&1; then
            command -v "$candidate"
            return
        fi
        if [[ -x "$candidate" ]]; then
            printf '%s\n' "$candidate"
            return
        fi
    done
}

clang_format="$(find_clang_format || true)"
if [[ -z "$clang_format" ]]; then
    cat >&2 <<'EOF'
error: clang-format 18.x not found.

Install it and expose it as clang-format-18. On Arch:
  sudo pacman -S clang18
  mkdir -p ~/.local/bin
  ln -sfn /usr/lib/llvm18/bin/clang-format ~/.local/bin/clang-format-18
EOF
    exit 127
fi

version="$("$clang_format" --version)"
case "$version" in
    *"version 18."*) ;;
    *)
        cat >&2 <<EOF
error: expected clang-format 18.x, got:
  $version

Use CLANG_FORMAT=/path/to/clang-format-18 or install clang-format-18 in PATH.
EOF
        exit 2
        ;;
esac

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT

if [[ "$#" -gt 0 ]]; then
    printf '%s\n' "$@" > "$tmp"
else
    git ls-files 'kmod/*.c' 'kmod/*.h' 'kmod/**/*.c' 'kmod/**/*.h' \
        'builtin/*.c' 'builtin/*.h' 'builtin/**/*.c' 'builtin/**/*.h' \
        | grep -Ev '(^kmod/third_party/|^kmod/generated/|\.mod\.c$)' > "$tmp"
fi

if [[ ! -s "$tmp" ]]; then
    exit 0
fi

case "$mode" in
    check)
        xargs "$clang_format" --dry-run --Werror < "$tmp"
        ;;
    fix)
        xargs "$clang_format" -i < "$tmp"
        ;;
esac
