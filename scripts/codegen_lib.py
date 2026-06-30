"""Shared scaffolding for the codegen scripts (codegen-interfaces.py and
codegen-hooks.py).

Both generators rendered the same boilerplate independently: the repo root, the
"AUTO-GENERATED … Regenerate with …" banner, the 11-segment path to the LSPosed
app's generated Kotlin package, and the write-if-changed + change-reporting tail
of main(). That duplication had already drifted (one script rewrote every file
unconditionally). Keep it here, imported by both, the way scripts/ already does
for changelog_lib.py / build_lib.py.
"""

from __future__ import annotations

from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent


def generated_header(source: str, regen_cmd: str) -> str:
    """The banner text for a generated file (without the per-language comment
    marker): ``AUTO-GENERATED from <source> … Regenerate with: <regen_cmd>``."""
    return f"AUTO-GENERATED from {source} — do not edit by hand. Regenerate with: {regen_cmd}"


def lsposed_generated_kt(filename: str, *, test: bool = False) -> Path:
    """Path to a generated Kotlin file under the LSPosed app's generated package
    (``…/src/{main,test}/kotlin/dev/okhsunrog/vpnhide/generated/<filename>``)."""
    source = "test" if test else "main"
    return (
        REPO_ROOT
        / "lsposed"
        / "app"
        / "src"
        / source
        / "kotlin"
        / "dev"
        / "okhsunrog"
        / "vpnhide"
        / "generated"
        / filename
    )


def write_if_changed(path: Path, content: str) -> bool:
    """Write ``content`` to ``path`` only if it differs (preserving mtime when it
    doesn't, so downstream Gradle/cargo don't needlessly rebuild). Returns True
    if the file was written."""
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists() and path.read_text(encoding="utf-8") == content:
        return False
    path.write_text(content, encoding="utf-8")
    return True


def emit_outputs(outputs: dict[Path, str]) -> int:
    """Write each {path: content} via [write_if_changed], report only what
    changed, and return 0 — the shared tail of both generators' main()."""
    changed = []
    for path, content in outputs.items():
        if write_if_changed(path, content):
            changed.append(path.relative_to(REPO_ROOT))

    if changed:
        print("Regenerated:")
        for p in changed:
            print(f"  {p}")
    else:
        print("All generated files already up to date.")
    return 0
