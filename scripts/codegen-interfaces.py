#!/usr/bin/env python3
"""Render the four iface-list matchers from data/interfaces.toml.

Generates one match function per target (kmod C, zygisk Rust,
lsposed/native Rust, lsposed Kotlin) so all four platforms agree on
which interface names are VPN tunnels. Re-run this script after
editing data/interfaces.toml and commit the regenerated files
alongside the toml change. CI's lint job re-runs the codegen and fails
on drift.

Stdlib only: tomllib (Python 3.11+) is the only non-builtin import,
and that's stdlib too.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path
from typing import Any

REPO_ROOT = Path(__file__).resolve().parent.parent
TOML_PATH = REPO_ROOT / "data" / "interfaces.toml"

# Output paths — kept next to the code that consumes them so include /
# import paths stay short and obvious.
OUT_KMOD = REPO_ROOT / "kmod" / "generated" / "iface_lists.h"
OUT_ZYGISK = REPO_ROOT / "zygisk" / "src" / "generated" / "iface_lists.rs"
OUT_LSP_NATIVE = REPO_ROOT / "lsposed" / "native" / "src" / "generated" / "iface_lists.rs"
OUT_LSP_KT = (
    REPO_ROOT
    / "lsposed"
    / "app"
    / "src"
    / "main"
    / "kotlin"
    / "dev"
    / "okhsunrog"
    / "vpnhide"
    / "generated"
    / "IfaceLists.kt"
)

GENERATED_HEADER_LINE = (
    "AUTO-GENERATED from data/interfaces.toml — do not edit by hand. "
    "Regenerate with: python3 scripts/codegen-interfaces.py"
)


# ---------------------------------------------------------------------------
# rule normalization
# ---------------------------------------------------------------------------


class Rule:
    """One match rule from the toml, normalized to a known kind.

    kind ∈ {"exact", "prefix", "prefix_digits", "contains"}.
    needle is the literal string (already lowercased — all targets
    fold case at match time).
    note is the human comment from the toml, copied into the
    generated source so reviewers see why each rule is there.
    """

    __slots__ = ("kind", "needle", "note")

    def __init__(self, kind: str, needle: str, note: str) -> None:
        self.kind = kind
        self.needle = needle
        self.note = note


def parse_rule(entry: dict[str, Any]) -> Rule:
    match = entry.get("match")
    if not isinstance(match, dict):
        raise SystemExit(f"entry missing or malformed `match`: {entry!r}")
    note = str(entry.get("note", "")).strip()

    keys = set(match.keys())
    if keys == {"exact"}:
        needle = str(match["exact"])
        kind = "exact"
    elif keys == {"prefix"}:
        needle = str(match["prefix"])
        kind = "prefix"
    elif keys == {"prefix", "suffix"}:
        if match["suffix"] != "digits":
            raise SystemExit(
                f"unsupported suffix {match['suffix']!r}; only 'digits' is implemented"
            )
        needle = str(match["prefix"])
        kind = "prefix_digits"
    elif keys == {"contains"}:
        needle = str(match["contains"])
        kind = "contains"
    else:
        raise SystemExit(f"unsupported match shape {keys!r} in entry {entry!r}")

    if not needle:
        raise SystemExit(f"empty needle in entry {entry!r}")
    if not all(0x20 <= ord(c) < 0x7F for c in needle):
        raise SystemExit(f"non-ASCII needle {needle!r} in entry {entry!r}")
    return Rule(kind, needle.lower(), note)


def load_rules() -> list[Rule]:
    with TOML_PATH.open("rb") as f:
        data = tomllib.load(f)
    raw = data.get("vpn") or []
    if not isinstance(raw, list) or not raw:
        raise SystemExit(f"{TOML_PATH}: missing or empty [[vpn]] table")
    return [parse_rule(e) for e in raw]


# ---------------------------------------------------------------------------
# emitters
# ---------------------------------------------------------------------------


def c_byte_lit(s: str) -> str:
    """Render a Python str as a C string literal, ASCII only."""
    return '"' + "".join(c if c not in '"\\' else "\\" + c for c in s) + '"'


def emit_kmod(rules: list[Rule]) -> str:
    """Render an inline header for the kernel module.

    Provides:
      static inline bool vpnhide_iface_is_vpn(const char *name);
    """
    lines: list[str] = []
    lines.append(f"/* {GENERATED_HEADER_LINE} */")
    lines.append("#ifndef VPNHIDE_GENERATED_IFACE_LISTS_H")
    lines.append("#define VPNHIDE_GENERATED_IFACE_LISTS_H")
    lines.append("")
    lines.append("#include <linux/string.h>")
    lines.append("#include <linux/ctype.h>")
    lines.append("#include <linux/types.h>")
    lines.append("")
    lines.append("static inline bool vpnhide_iface_starts_with_ci(")
    lines.append("\tconst char *name, const char *prefix)")
    lines.append("{")
    lines.append("\tsize_t i;")
    lines.append("\tfor (i = 0; prefix[i]; i++) {")
    lines.append("\t\tif (!name[i])")
    lines.append("\t\t\treturn false;")
    lines.append("\t\tif (tolower((unsigned char)name[i]) !=")
    lines.append("\t\t    (unsigned char)prefix[i])")
    lines.append("\t\t\treturn false;")
    lines.append("\t}")
    lines.append("\treturn true;")
    lines.append("}")
    lines.append("")
    lines.append("static inline bool vpnhide_iface_starts_with_then_digits_ci(")
    lines.append("\tconst char *name, const char *prefix)")
    lines.append("{")
    lines.append("\tsize_t i;")
    lines.append("\tif (!vpnhide_iface_starts_with_ci(name, prefix))")
    lines.append("\t\treturn false;")
    lines.append("\ti = strlen(prefix);")
    lines.append("\tif (!name[i])")
    lines.append("\t\treturn false;")
    lines.append("\tfor (; name[i]; i++)")
    lines.append("\t\tif (name[i] < '0' || name[i] > '9')")
    lines.append("\t\t\treturn false;")
    lines.append("\treturn true;")
    lines.append("}")
    lines.append("")
    lines.append("static inline bool vpnhide_iface_equals_ci(")
    lines.append("\tconst char *name, const char *other)")
    lines.append("{")
    lines.append("\tsize_t i;")
    lines.append("\tfor (i = 0; other[i]; i++) {")
    lines.append("\t\tif (!name[i])")
    lines.append("\t\t\treturn false;")
    lines.append("\t\tif (tolower((unsigned char)name[i]) !=")
    lines.append("\t\t    (unsigned char)other[i])")
    lines.append("\t\t\treturn false;")
    lines.append("\t}")
    lines.append("\treturn name[i] == '\\0';")
    lines.append("}")
    lines.append("")
    lines.append("static inline bool vpnhide_iface_contains_ci(")
    lines.append("\tconst char *name, const char *needle)")
    lines.append("{")
    lines.append("\tsize_t nlen = strlen(needle);")
    lines.append("\tsize_t i, j;")
    lines.append("\tif (nlen == 0)")
    lines.append("\t\treturn true;")
    lines.append("\tfor (i = 0; name[i]; i++) {")
    lines.append("\t\tfor (j = 0; j < nlen; j++) {")
    lines.append("\t\t\tif (!name[i + j])")
    lines.append("\t\t\t\treturn false;")
    lines.append("\t\t\tif (tolower((unsigned char)name[i + j]) !=")
    lines.append("\t\t\t    (unsigned char)needle[j])")
    lines.append("\t\t\t\tbreak;")
    lines.append("\t\t}")
    lines.append("\t\tif (j == nlen)")
    lines.append("\t\t\treturn true;")
    lines.append("\t}")
    lines.append("\treturn false;")
    lines.append("}")
    lines.append("")
    lines.append("static inline bool vpnhide_iface_is_vpn(const char *name)")
    lines.append("{")
    lines.append("\tif (!name || !name[0])")
    lines.append("\t\treturn false;")
    for r in rules:
        comment = f"\t/* {r.note} */" if r.note else ""
        if comment:
            lines.append(comment)
        if r.kind == "exact":
            lines.append(
                f"\tif (vpnhide_iface_equals_ci(name, {c_byte_lit(r.needle)}))"
            )
        elif r.kind == "prefix":
            lines.append(
                f"\tif (vpnhide_iface_starts_with_ci(name, {c_byte_lit(r.needle)}))"
            )
        elif r.kind == "prefix_digits":
            lines.append(
                f"\tif (vpnhide_iface_starts_with_then_digits_ci(name, {c_byte_lit(r.needle)}))"
            )
        elif r.kind == "contains":
            lines.append(
                f"\tif (vpnhide_iface_contains_ci(name, {c_byte_lit(r.needle)}))"
            )
        lines.append("\t\treturn true;")
    lines.append("\treturn false;")
    lines.append("}")
    lines.append("")
    lines.append("#endif /* VPNHIDE_GENERATED_IFACE_LISTS_H */")
    lines.append("")
    return "\n".join(lines)


def rust_byte_lit(s: str) -> str:
    return 'b"' + "".join(c if c not in '"\\' else "\\" + c for c in s) + '"'


def emit_rust(rules: list[Rule]) -> str:
    """Render a Rust module exporting `pub fn matches_vpn(name: &[u8]) -> bool`.

    Used by both zygisk and lsposed/native (identical body) so that
    self-test and the actual hooks share one definition.
    """
    lines: list[str] = []
    lines.append(f"// {GENERATED_HEADER_LINE}")
    lines.append("")
    lines.append("#![allow(dead_code)]")
    lines.append("")
    lines.append("fn starts_with_ci(name: &[u8], prefix: &[u8]) -> bool {")
    lines.append("    if name.len() < prefix.len() {")
    lines.append("        return false;")
    lines.append("    }")
    lines.append("    for (i, &p) in prefix.iter().enumerate() {")
    lines.append("        if name[i].to_ascii_lowercase() != p {")
    lines.append("            return false;")
    lines.append("        }")
    lines.append("    }")
    lines.append("    true")
    lines.append("}")
    lines.append("")
    lines.append("fn starts_with_then_digits_ci(name: &[u8], prefix: &[u8]) -> bool {")
    lines.append("    if !starts_with_ci(name, prefix) {")
    lines.append("        return false;")
    lines.append("    }")
    lines.append("    let rest = &name[prefix.len()..];")
    lines.append("    !rest.is_empty() && rest.iter().all(|b| b.is_ascii_digit())")
    lines.append("}")
    lines.append("")
    lines.append("fn equals_ci(name: &[u8], other: &[u8]) -> bool {")
    lines.append("    if name.len() != other.len() {")
    lines.append("        return false;")
    lines.append("    }")
    lines.append("    name.iter()")
    lines.append("        .zip(other.iter())")
    lines.append("        .all(|(a, b)| a.to_ascii_lowercase() == *b)")
    lines.append("}")
    lines.append("")
    lines.append("fn contains_ci(haystack: &[u8], needle: &[u8]) -> bool {")
    lines.append("    if needle.is_empty() {")
    lines.append("        return true;")
    lines.append("    }")
    lines.append("    if needle.len() > haystack.len() {")
    lines.append("        return false;")
    lines.append("    }")
    lines.append("    for start in 0..=haystack.len() - needle.len() {")
    lines.append("        let window = &haystack[start..start + needle.len()];")
    lines.append("        if window")
    lines.append("            .iter()")
    lines.append("            .zip(needle.iter())")
    lines.append("            .all(|(a, b)| a.eq_ignore_ascii_case(b))")
    lines.append("        {")
    lines.append("            return true;")
    lines.append("        }")
    lines.append("    }")
    lines.append("    false")
    lines.append("}")
    lines.append("")
    lines.append("/// True if the name matches any VPN-iface rule from data/interfaces.toml.")
    lines.append("pub fn matches_vpn(name: &[u8]) -> bool {")
    lines.append("    if name.is_empty() {")
    lines.append("        return false;")
    lines.append("    }")
    for r in rules:
        if r.note:
            lines.append(f"    // {r.note}")
        if r.kind == "exact":
            fn = "equals_ci"
        elif r.kind == "prefix":
            fn = "starts_with_ci"
        elif r.kind == "prefix_digits":
            fn = "starts_with_then_digits_ci"
        elif r.kind == "contains":
            fn = "contains_ci"
        lines.append(f"    if {fn}(name, {rust_byte_lit(r.needle)}) {{")
        lines.append("        return true;")
        lines.append("    }")
    lines.append("    false")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


def kt_str_lit(s: str) -> str:
    return '"' + "".join(c if c not in '"\\' else "\\" + c for c in s) + '"'


def emit_kotlin(rules: list[Rule]) -> str:
    """Render a Kotlin singleton with `IfaceLists.isVpnIface(name)`."""
    lines: list[str] = []
    lines.append(f"// {GENERATED_HEADER_LINE}")
    lines.append("")
    lines.append("package dev.okhsunrog.vpnhide.generated")
    lines.append("")
    lines.append("internal object IfaceLists {")
    lines.append("    /** True if `name` looks like a VPN tunnel per data/interfaces.toml. */")
    lines.append("    fun isVpnIface(name: String): Boolean {")
    lines.append("        if (name.isEmpty()) return false")
    lines.append("        val n = name.lowercase()")
    for r in rules:
        if r.note:
            lines.append(f"        // {r.note}")
        if r.kind == "exact":
            cond = f"n == {kt_str_lit(r.needle)}"
        elif r.kind == "prefix":
            cond = f"n.startsWith({kt_str_lit(r.needle)})"
        elif r.kind == "prefix_digits":
            lit = kt_str_lit(r.needle)
            cond = (
                f"n.startsWith({lit}) && "
                f"n.length > {len(r.needle)} && "
                f"n.substring({len(r.needle)}).all {{ it.isDigit() }}"
            )
        elif r.kind == "contains":
            cond = f"n.contains({kt_str_lit(r.needle)})"
        lines.append(f"        if ({cond}) return true")
    lines.append("        return false")
    lines.append("    }")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def write_if_changed(path: Path, content: str) -> bool:
    path.parent.mkdir(parents=True, exist_ok=True)
    if path.exists() and path.read_text(encoding="utf-8") == content:
        return False
    path.write_text(content, encoding="utf-8")
    return True


def main() -> int:
    rules = load_rules()
    outputs = {
        OUT_KMOD: emit_kmod(rules),
        OUT_ZYGISK: emit_rust(rules),
        OUT_LSP_NATIVE: emit_rust(rules),
        OUT_LSP_KT: emit_kotlin(rules),
    }
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


if __name__ == "__main__":
    sys.exit(main())
