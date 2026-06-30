#!/usr/bin/env python3
"""Render the four iface-list matchers + their unit tests from
data/interfaces.toml.

Generates one match function per target (kmod C, zygisk Rust,
lsposed/native Rust, lsposed Kotlin) so all four platforms agree on
which interface names are VPN tunnels, plus per-language test files
seeded from the same [[test]] vectors so CI catches drift instantly.

Re-run after editing data/interfaces.toml and commit the regenerated
files. CI's lint job re-runs the codegen and fails on drift.

Stdlib only: tomllib (Python 3.11+) is the only non-builtin import,
and that's stdlib too.
"""

from __future__ import annotations

import sys
import tomllib
from pathlib import Path
from typing import Any

sys.path.insert(0, str(Path(__file__).resolve().parent))
from codegen_lib import (  # type: ignore[import-not-found]
    REPO_ROOT,
    emit_outputs,
    generated_header,
    lsposed_generated_kt,
)

TOML_PATH = REPO_ROOT / "data" / "interfaces.toml"

# Output paths — kept next to the code that consumes them so include /
# import paths stay short and obvious.
OUT_KMOD = REPO_ROOT / "kmod" / "generated" / "iface_lists.h"
OUT_KMOD_TEST = REPO_ROOT / "kmod" / "test_iface_lists.c"
OUT_ZYGISK = REPO_ROOT / "zygisk" / "src" / "generated" / "iface_lists.rs"
OUT_LSP_NATIVE = REPO_ROOT / "lsposed" / "native" / "src" / "generated" / "iface_lists.rs"
OUT_LSP_KT = lsposed_generated_kt("IfaceLists.kt")
OUT_LSP_KT_TEST = lsposed_generated_kt("IfaceListsGeneratedTest.kt", test=True)

GENERATED_HEADER_LINE = generated_header(
    "data/interfaces.toml", "python3 scripts/codegen-interfaces.py"
)


# ---------------------------------------------------------------------------
# rule normalization
# ---------------------------------------------------------------------------


VALID_KINDS = ("exact", "prefix", "prefix_digits", "contains")


class Rule:
    """One match rule from the toml, normalized to a known kind.

    kind ∈ VALID_KINDS.
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


class TestVector:
    __slots__ = ("name", "is_vpn")

    def __init__(self, name: str, is_vpn: bool) -> None:
        self.name = name
        self.is_vpn = is_vpn


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
        suffix = str(match["suffix"])
        if suffix == "digits":
            kind = "prefix_digits"
        else:
            raise SystemExit(f"unsupported suffix {suffix!r}; expected digits")
        needle = str(match["prefix"])
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


def parse_test(entry: dict[str, Any]) -> TestVector:
    if "name" not in entry or "is_vpn" not in entry:
        raise SystemExit(f"[[test]] entry needs name and is_vpn: {entry!r}")
    name = str(entry["name"])
    if not all(0x20 <= ord(c) < 0x7F for c in name):
        raise SystemExit(f"non-ASCII test name {name!r}; the matcher itself is ASCII-only")
    return TestVector(name=name, is_vpn=bool(entry["is_vpn"]))


def load() -> tuple[list[Rule], list[TestVector]]:
    with TOML_PATH.open("rb") as f:
        data = tomllib.load(f)
    raw_rules = data.get("vpn") or []
    if not isinstance(raw_rules, list) or not raw_rules:
        raise SystemExit(f"{TOML_PATH}: missing or empty [[vpn]] table")
    rules = [parse_rule(e) for e in raw_rules]
    tests = [parse_test(e) for e in (data.get("test") or [])]
    return rules, tests


# ---------------------------------------------------------------------------
# escape helpers
# ---------------------------------------------------------------------------


def c_str_lit(s: str) -> str:
    out = ['"']
    for c in s:
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif 0x20 <= ord(c) < 0x7F:
            out.append(c)
        else:
            out.append(f"\\x{ord(c):02x}")
    out.append('"')
    return "".join(out)


def rust_byte_lit(s: str) -> str:
    # Use byte-string with escapes so non-printable / quotes survive.
    out = ['b"']
    for c in s:
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif 0x20 <= ord(c) < 0x7F:
            out.append(c)
        else:
            out.append(f"\\x{ord(c):02x}")
    out.append('"')
    return "".join(out)


def kt_str_lit(s: str) -> str:
    out = ['"']
    for c in s:
        if c == '"':
            out.append('\\"')
        elif c == "\\":
            out.append("\\\\")
        elif c == "$":
            out.append("\\$")
        elif 0x20 <= ord(c) < 0x7F:
            out.append(c)
        else:
            out.append(f"\\u{ord(c):04x}")
    out.append('"')
    return "".join(out)


# ---------------------------------------------------------------------------
# C emitter (kmod)
# ---------------------------------------------------------------------------


# Self-contained C matcher helpers (no libc/kernel headers). Literal template
# block — `\t` becomes a tab in the generated header (kernel style), same as
# the rest of the emitter. Shared verbatim by the .ko and the KPM backend.
KMOD_MATCHER_HELPERS = """\
static inline char vpnhide__lc(char c)
{
\treturn (c >= 'A' && c <= 'Z') ? (char)(c - 'A' + 'a') : c;
}

static inline int vpnhide_iface_starts_with_ci(const char *name, const char *prefix)
{
\tint i;
\tfor (i = 0; prefix[i]; i++) {
\t\tif (!name[i])
\t\t\treturn 0;
\t\tif (vpnhide__lc(name[i]) != vpnhide__lc(prefix[i]))
\t\t\treturn 0;
\t}
\treturn 1;
}

static inline int vpnhide_iface_starts_with_then_digits_ci(const char *name,
\t\t\t\t\t\t\t   const char *prefix)
{
\tint i = 0;
\tif (!vpnhide_iface_starts_with_ci(name, prefix))
\t\treturn 0;
\twhile (prefix[i])
\t\ti++;
\tif (!name[i])
\t\treturn 0;
\tfor (; name[i]; i++)
\t\tif (name[i] < '0' || name[i] > '9')
\t\t\treturn 0;
\treturn 1;
}

static inline int vpnhide_iface_equals_ci(const char *name, const char *other)
{
\tint i;
\tfor (i = 0; other[i]; i++) {
\t\tif (!name[i])
\t\t\treturn 0;
\t\tif (vpnhide__lc(name[i]) != vpnhide__lc(other[i]))
\t\t\treturn 0;
\t}
\treturn name[i] == '\\0';
}

static inline int vpnhide_iface_contains_ci(const char *name, const char *needle)
{
\tint i, j;
\tif (!needle[0])
\t\treturn 1;
\tfor (i = 0; name[i]; i++) {
\t\tfor (j = 0; needle[j]; j++) {
\t\t\tif (!name[i + j])
\t\t\t\treturn 0;
\t\t\tif (vpnhide__lc(name[i + j]) != vpnhide__lc(needle[j]))
\t\t\t\tbreak;
\t\t}
\t\tif (!needle[j])
\t\t\treturn 1;
\t}
\treturn 0;
}
"""


def emit_kmod(rules: list[Rule]) -> str:
    """Render an inline header for the kernel module.

    Header is dual-target: builds in kernel (default) AND in userspace
    when __VPNHIDE_HOST_TEST is defined, so test_iface_lists.c can link
    the same matcher.
    """
    lines: list[str] = []
    lines.append(f"/* {GENERATED_HEADER_LINE} */")
    lines.append("#ifndef VPNHIDE_GENERATED_IFACE_LISTS_H")
    lines.append("#define VPNHIDE_GENERATED_IFACE_LISTS_H")
    lines.append("")
    # Self-contained matcher: no <ctype.h>/<string.h>/<stdbool.h>/<linux/*>,
    # so the SAME header compiles in every world that consumes it — the
    # kretprobe .ko (real kernel headers), the KernelPatch KPM (-nostdinc, no
    # libc/ctype stubs), and the host unit test. ASCII case-fold and length
    # are done locally; returns int (0/1).
    lines.append(KMOD_MATCHER_HELPERS)
    lines.append("static inline int vpnhide_iface_is_vpn(const char *name)")
    lines.append("{")
    lines.append("\tif (!name || !name[0])")
    lines.append("\t\treturn 0;")
    fn_for_kind = {
        "exact": "vpnhide_iface_equals_ci",
        "prefix": "vpnhide_iface_starts_with_ci",
        "prefix_digits": "vpnhide_iface_starts_with_then_digits_ci",
        "contains": "vpnhide_iface_contains_ci",
    }
    for r in rules:
        if r.note:
            lines.append(f"\t/* {r.note} */")
        lines.append(f"\tif ({fn_for_kind[r.kind]}(name, {c_str_lit(r.needle)}))")
        lines.append("\t\treturn 1;")
    lines.append("\treturn 0;")
    lines.append("}")
    lines.append("")
    lines.append("#endif /* VPNHIDE_GENERATED_IFACE_LISTS_H */")
    lines.append("")
    return "\n".join(lines)


def emit_kmod_test(tests: list[TestVector]) -> str:
    """Render a userspace test driver.

    Built in CI's lint job (gcc on the host). Includes the same
    iface_lists.h that the kernel module uses, but compiles with
    libc headers via the !__KERNEL__ branch of the header guard.
    """
    lines: list[str] = []
    lines.append(f"/* {GENERATED_HEADER_LINE} */")
    lines.append("/*")
    lines.append(" * Userspace test driver for generated/iface_lists.h.")
    lines.append(" * Build: gcc -O2 -Wall -Werror -o test_iface_lists test_iface_lists.c")
    lines.append(" * Run: ./test_iface_lists  (exit 0 on success, 1 on failure)")
    lines.append(" */")
    lines.append("")
    lines.append("#include <stdbool.h>")
    lines.append("#include <stdio.h>")
    lines.append("")
    lines.append('#include "generated/iface_lists.h"')
    lines.append("")
    lines.append("static int failures;")
    lines.append("")
    lines.append("static void check(const char *name, bool expected)")
    lines.append("{")
    lines.append("\tbool got = vpnhide_iface_is_vpn(name);")
    lines.append("\tif (got != expected) {")
    lines.append("\t\tfprintf(stderr,")
    lines.append('\t\t\t"FAIL: vpnhide_iface_is_vpn(\\"%s\\") = %s, expected %s\\n",')
    lines.append('\t\t\tname, got ? "true" : "false",')
    lines.append('\t\t\texpected ? "true" : "false");')
    lines.append("\t\tfailures++;")
    lines.append("\t}")
    lines.append("}")
    lines.append("")
    lines.append("int main(void)")
    lines.append("{")
    for t in tests:
        expected = "true" if t.is_vpn else "false"
        lines.append(f"\tcheck({c_str_lit(t.name)}, {expected});")
    lines.append("")
    lines.append("\tif (failures) {")
    lines.append('\t\tfprintf(stderr, "%d test(s) failed\\n", failures);')
    lines.append("\t\treturn 1;")
    lines.append("\t}")
    lines.append(f'\tprintf("OK: {len(tests)} vectors passed\\n");')
    lines.append("\treturn 0;")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Rust emitter (zygisk + lsposed/native — same body)
# ---------------------------------------------------------------------------


def emit_rust(rules: list[Rule], tests: list[TestVector]) -> str:
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
    # Test module — generated assertions are wide; skip rustfmt rather
    # than wrap each assertion across 5 lines.
    lines.append("#[cfg(test)]")
    lines.append("#[rustfmt::skip]")
    lines.append("mod tests {")
    lines.append("    use super::*;")
    lines.append("")
    lines.append("    #[test]")
    lines.append("    fn generated_vectors() {")
    for t in tests:
        # `assert!(x, msg)` / `assert!(!x, msg)` instead of
        # `assert_eq!(x, true/false, msg)` — clippy::bool_assert_comparison
        # would otherwise fire on every generated row when contributors
        # run `cargo clippy --tests`.
        prefix = "" if t.is_vpn else "!"
        lines.append(
            f"        assert!({prefix}matches_vpn({rust_byte_lit(t.name)}), "
            f'"matches_vpn({t.name!r})");'
        )
    lines.append("    }")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# Kotlin emitter (production code + separate test class)
# ---------------------------------------------------------------------------


def emit_kotlin(rules: list[Rule]) -> str:
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


def emit_kotlin_test(tests: list[TestVector]) -> str:
    lines: list[str] = []
    lines.append(f"// {GENERATED_HEADER_LINE}")
    lines.append("")
    lines.append("package dev.okhsunrog.vpnhide.generated")
    lines.append("")
    lines.append("import org.junit.Assert.assertEquals")
    lines.append("import org.junit.Test")
    lines.append("")
    lines.append("class IfaceListsGeneratedTest {")
    lines.append("    @Test")
    lines.append("    fun `generated vectors`() {")
    for t in tests:
        expected = "true" if t.is_vpn else "false"
        lines.append(
            f"        assertEquals({kt_str_lit(t.name)}, {expected}, "
            f"IfaceLists.isVpnIface({kt_str_lit(t.name)}))"
        )
    lines.append("    }")
    lines.append("}")
    lines.append("")
    return "\n".join(lines)


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def main() -> int:
    rules, tests = load()
    rust_body = emit_rust(rules, tests)
    return emit_outputs(
        {
            OUT_KMOD: emit_kmod(rules),
            OUT_KMOD_TEST: emit_kmod_test(tests),
            OUT_ZYGISK: rust_body,
            OUT_LSP_NATIVE: rust_body,
            OUT_LSP_KT: emit_kotlin(rules),
            OUT_LSP_KT_TEST: emit_kotlin_test(tests),
        }
    )


if __name__ == "__main__":
    sys.exit(main())
