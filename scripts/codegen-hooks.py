#!/usr/bin/env -S uv run
#
# /// script
# requires-python = ">=3.12"
# ///
"""Render the hook-id registry + status error codes from data/hooks.toml.

The registry is the global id space shared by the control/stats protocol
(docs/protocol.md §5): bit N of a config mask == hook id N == the id used in a
stats line. This script emits the matching id enums, per-backend "own" masks,
and status error codes for every language that touches the protocol:

  - kmod/KPM   C   -> kmod/generated/hook_ids.h
  - protocol   Rust-> crates/protocol/src/generated/hook_ids.rs
  - zygisk     Rust-> zygisk/src/generated/hook_ids.rs
  - lsposed    Rust-> lsposed/native/src/generated/hook_ids.rs
  - app        Kotlin-> .../generated/HookIds.kt

Re-run after editing data/hooks.toml and commit the regenerated files. CI's
lint job re-runs the codegen and fails on drift, so the numbering can never
diverge between backends.

Run via `uv run scripts/codegen-hooks.py` (or `./scripts/codegen-hooks.py` — the
uv shebang provisions a pinned interpreter). No external deps: tomllib is stdlib
and the emitters are plain string building.
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

TOML_PATH = REPO_ROOT / "data" / "hooks.toml"

# Targets are only the protocol participants (§1.3): the kernel backends (C),
# the Zygisk backend (Rust), and the app + system_server hook (Kotlin). NOT
# lsposed/native — that Rust crate is the diagnostic-probe cdylib; it
# consumes iface_lists (VPN-name matching) but never the protocol/registry.
OUT_KMOD = REPO_ROOT / "kmod" / "generated" / "hook_ids.h"
OUT_PROTOCOL_RS = REPO_ROOT / "crates" / "protocol" / "src" / "generated" / "hook_ids.rs"
OUT_ZYGISK = REPO_ROOT / "zygisk" / "src" / "generated" / "hook_ids.rs"
OUT_LSP_KT = lsposed_generated_kt("HookIds.kt")

GENERATED_HEADER_LINE = generated_header("data/hooks.toml", "uv run scripts/codegen-hooks.py")

KNOWN_BACKENDS = ("kernel", "kmod", "kpm", "kpatch", "zygisk", "lsposed")


# ---------------------------------------------------------------------------
# load + validate
# ---------------------------------------------------------------------------


class Hook:
    def __init__(self, raw: dict[str, Any]) -> None:
        self.id: int = raw["id"]
        self.name: str = raw["name"]
        backend = raw.get("backend")
        backends = raw.get("backends")
        if (backend is None) == (backends is None):
            sys.exit(
                f"error: hook {self.name!r} must define exactly one of 'backend' or 'backends'"
            )
        self.backends: tuple[str, ...] = (backend,) if backend is not None else tuple(backends)
        if not self.backends or len(set(self.backends)) != len(self.backends):
            sys.exit(f"error: hook {self.name!r} has invalid backends {self.backends!r}")
        self.note: str = raw.get("note", "")


class Err:
    def __init__(self, raw: dict[str, Any]) -> None:
        self.id: int = raw["id"]
        self.name: str = raw["name"]
        self.note: str = raw.get("note", "")


class Backend:
    def __init__(self, raw: dict[str, Any]) -> None:
        self.id: int = raw["id"]
        self.name: str = raw["name"]
        self.note: str = raw.get("note", "")


def load() -> tuple[list[Hook], list[Err], list[Backend]]:
    with TOML_PATH.open("rb") as fh:
        data = tomllib.load(fh)
    hooks = [Hook(h) for h in data.get("hook", [])]
    errs = [Err(e) for e in data.get("error", [])]
    backends = [Backend(b) for b in data.get("backend", [])]

    # ids must be dense, append-only, and 0-based — anything else is a bug
    # that would silently shift the global id space.
    for label, items in (("hook", hooks), ("error", errs), ("backend", backends)):
        ids = [it.id for it in items]
        if ids != list(range(len(ids))):
            sys.exit(f"error: {label} ids must be 0..N-1 with no gaps, got {ids}")
        names = [it.name for it in items]
        if len(set(names)) != len(names):
            sys.exit(f"error: duplicate {label} name in {names}")
    for h in hooks:
        unknown = set(h.backends) - set(KNOWN_BACKENDS)
        if unknown:
            sys.exit(f"error: hook {h.name!r} has unknown backends {sorted(unknown)!r}")
    return hooks, errs, backends


def backend_mask(hooks: list[Hook], backend: str) -> int:
    m = 0
    for h in hooks:
        if backend in h.backends:
            m |= 1 << h.id
    return m


def append_dense_hook_mapping(
    lines: list[str], name: str, hooks: list[Hook], description: str
) -> None:
    """Emit dense in-memory slots while preserving global hook IDs on wire."""
    symbol = upper(name)
    lines.append(f"/* {description}")
    lines.append("   The wire keeps global hook ids; these helpers only compact the")
    lines.append("   backend's in-memory counters. */")
    lines.append(f"#define VPNHIDE_{symbol}_HOOK_COUNT {len(hooks)}")
    lines.append(f"static inline int vpnhide_{name}_hook_slot(enum vpnhide_hook_id id)")
    lines.append("{")
    lines.append("\tswitch (id) {")
    for slot, hook in enumerate(hooks):
        lines.append(f"\tcase VPNHIDE_HOOK_{upper(hook.name)}: return {slot};")
    lines.append("\tdefault: return -1;")
    lines.append("\t}")
    lines.append("}")
    lines.append("")
    lines.append(f"static inline enum vpnhide_hook_id vpnhide_{name}_hook_id(unsigned int slot)")
    lines.append("{")
    lines.append("\tswitch (slot) {")
    for slot, hook in enumerate(hooks):
        lines.append(f"\tcase {slot}: return VPNHIDE_HOOK_{upper(hook.name)};")
    lines.append("\tdefault: return VPNHIDE_HOOK_COUNT;")
    lines.append("\t}")
    lines.append("}")
    lines.append("")


# name-casing helpers ------------------------------------------------------


def upper(name: str) -> str:
    return name.upper()


def pascal(name: str) -> str:
    return "".join(part.capitalize() for part in name.split("_"))


def kt_string(value: str) -> str:
    return value.replace("\\", "\\\\").replace('"', '\\"')


# ---------------------------------------------------------------------------
# emitters
# ---------------------------------------------------------------------------


def emit_kmod(hooks: list[Hook], errs: list[Err], backends: list[Backend]) -> str:
    L: list[str] = [f"/* {GENERATED_HEADER_LINE} */"]
    L.append("#ifndef VPNHIDE_GENERATED_HOOK_IDS_H")
    L.append("#define VPNHIDE_GENERATED_HOOK_IDS_H")
    L.append("")
    L.append("/* Global hook id space (data/hooks.toml). bit N == hook id N. */")
    width = max(len(f"VPNHIDE_HOOK_{upper(h.name)}") for h in hooks)
    L.append("enum vpnhide_hook_id {")
    for h in hooks:
        name = f"VPNHIDE_HOOK_{upper(h.name)}"
        L.append(f"\t{name:<{width}} = {h.id},")
    L.append(f"\t{'VPNHIDE_HOOK_COUNT':<{width}} = {len(hooks)},")
    L.append("};")
    L.append("")
    L.append("static inline unsigned int vpnhide_hook_bit(enum vpnhide_hook_id id)")
    L.append("{")
    L.append("\treturn 1u << (unsigned int)id;")
    L.append("}")
    L.append("")
    L.append("/* Hooks owned by each backend: apply `mask & own`, ignore foreign bits. */")
    for b in KNOWN_BACKENDS:
        L.append(f"#define VPNHIDE_{upper(b)}_HOOK_MASK 0x{backend_mask(hooks, b):x}u")
    L.append("")
    kernel_hooks = [h for h in hooks if "kernel" in h.backends]
    append_dense_hook_mapping(
        L,
        "kernel",
        kernel_hooks,
        "Dense stats slots shared by the .ko and KPM kernel hooks.",
    )
    kmod_hooks = [h for h in hooks if set(h.backends) & {"kernel", "kmod"}]
    append_dense_hook_mapping(
        L,
        "kmod_stats",
        kmod_hooks,
        "Dense stats slots for every hook the .ko can install.",
    )
    kpm_hooks = [h for h in hooks if set(h.backends) & {"kernel", "kpm"}]
    append_dense_hook_mapping(
        L,
        "kpm_stats",
        kpm_hooks,
        "Dense stats slots for every hook the KPM can install.",
    )
    L.append("/* status error codes (protocol §5.1). */")
    ewidth = max(len(f"VPNHIDE_ERR_{upper(e.name)}") for e in errs)
    L.append("enum vpnhide_status_error {")
    for e in errs:
        name = f"VPNHIDE_ERR_{upper(e.name)}"
        L.append(f"\t{name:<{ewidth}} = {e.id},")
    L.append("};")
    L.append("")
    L.append("/* backend ids (protocol §4.3 `status backend <id>`). */")
    bwidth = max(len(f"VPNHIDE_BACKEND_{upper(b.name)}") for b in backends)
    L.append("enum vpnhide_backend {")
    for b in backends:
        name = f"VPNHIDE_BACKEND_{upper(b.name)}"
        L.append(f"\t{name:<{bwidth}} = {b.id},")
    L.append("};")
    L.append("")
    L.append("/* Hook name for an id (labeling / debug). Inline so the header stays")
    L.append("   self-contained and an unused table never warns. */")
    L.append("static inline const char *vpnhide_hook_name(enum vpnhide_hook_id id)")
    L.append("{")
    L.append("\tswitch (id) {")
    for h in hooks:
        L.append(f'\tcase VPNHIDE_HOOK_{upper(h.name)}: return "{h.name}";')
    L.append('\tdefault: return "?";')
    L.append("\t}")
    L.append("}")
    L.append("")
    L.append("#endif /* VPNHIDE_GENERATED_HOOK_IDS_H */")
    L.append("")
    return "\n".join(L)


def emit_rust(hooks: list[Hook], errs: list[Err], backends: list[Backend]) -> str:
    L: list[str] = [f"// {GENERATED_HEADER_LINE}", "", "#![allow(dead_code)]", ""]
    L.append("/// Global hook id space (data/hooks.toml). bit N == hook id N.")
    L.append("#[repr(u32)]")
    L.append("#[derive(Copy, Clone, Eq, PartialEq, Debug)]")
    L.append("pub enum Hook {")
    for h in hooks:
        if h.note:
            L.append(f"    /// {h.note}")
        L.append(f"    {pascal(h.name)} = {h.id},")
    L.append("}")
    L.append("")
    L.append("impl Hook {")
    L.append("    /// This hook's bit in the control/stats wire mask.")
    L.append("    pub const fn bit(self) -> u32 {")
    L.append("        1u32 << self as u32")
    L.append("    }")
    L.append("")
    L.append("    /// This hook's canonical config name.")
    L.append("    pub const fn name(self) -> &'static str {")
    L.append("        match self {")
    for h in hooks:
        L.append(f'            Self::{pascal(h.name)} => "{h.name}",')
    L.append("        }")
    L.append("    }")
    L.append("")
    L.append("    /// Resolve a canonical config hook name.")
    L.append("    pub fn from_name(name: &str) -> Option<Self> {")
    L.append("        match name {")
    for h in hooks:
        L.append(f'            "{h.name}" => Some(Self::{pascal(h.name)}),')
    L.append("            _ => None,")
    L.append("        }")
    L.append("    }")
    L.append("}")
    L.append("")
    L.append(f"pub const HOOK_COUNT: u32 = {len(hooks)};")
    L.append("")
    L.append("/// Hooks owned by each backend: apply `mask & own`.")
    for b in KNOWN_BACKENDS:
        L.append(f"pub const {upper(b)}_HOOK_MASK: u32 = 0x{backend_mask(hooks, b):x};")
    L.append("")
    L.append("/// status error codes (protocol §5.1).")
    L.append("#[repr(u32)]")
    L.append("#[derive(Copy, Clone, Eq, PartialEq, Debug)]")
    L.append("pub enum StatusError {")
    for e in errs:
        if e.note:
            L.append(f"    /// {e.note}")
        L.append(f"    {pascal(e.name)} = {e.id},")
    L.append("}")
    L.append("")
    L.append("/// backend ids (protocol §4.3 `status backend <id>`).")
    L.append("#[repr(u32)]")
    L.append("#[derive(Copy, Clone, Eq, PartialEq, Debug)]")
    L.append("pub enum Backend {")
    for b in backends:
        if b.note:
            L.append(f"    /// {b.note}")
        L.append(f"    {pascal(b.name)} = {b.id},")
    L.append("}")
    L.append("")
    return "\n".join(L)


def emit_kotlin(hooks: list[Hook], errs: list[Err], backends: list[Backend]) -> str:
    # Shaped to match `ktlint --format` (lsposed's quality gate): multiline
    # class signatures with trailing comma, a blank line between commented enum
    # entries, no trailing `;`. Keep this in sync if the ktlint style changes.
    L: list[str] = [f"// {GENERATED_HEADER_LINE}", ""]
    L.append("package dev.okhsunrog.vpnhide.generated")
    L.append("")
    L.append("import kotlinx.serialization.Serializable")
    L.append("")
    L.append("/** Global hook id space (data/hooks.toml). bit N == hook id N. */")
    L.append("internal object HookIds {")
    L.append("    // @Serializable: Hook is embedded in the canonical VpnHideState (diagnostics")
    L.append("    // report + dashboard optional-hook set) that serializes straight to the debug")
    L.append("    // JSON; the enum serializes by entry name.")
    L.append("    @Serializable")
    L.append("    enum class Hook(")
    L.append("        val id: Int,")
    L.append("        val hookName: String,")
    L.append("        val note: String,")
    L.append("    ) {")
    for i, h in enumerate(hooks):
        if i:
            L.append("")
        if h.note:
            L.append(f"        // {h.note}")
        L.append(f'        {upper(h.name)}({h.id}, "{h.name}", "{kt_string(h.note)}"),')
    L.append("    }")
    L.append("")
    L.append("    // Hooks owned by each backend: apply `mask and own`.")
    for b in KNOWN_BACKENDS:
        L.append(f"    const val {upper(b)}_HOOK_MASK = 0x{backend_mask(hooks, b):x}")
    L.append("")
    L.append("    /** status error codes (protocol §5.1). */")
    L.append("    enum class StatusError(")
    L.append("        val code: Int,")
    L.append("    ) {")
    for i, e in enumerate(errs):
        if i:
            L.append("")
        if e.note:
            L.append(f"        // {e.note}")
        L.append(f"        {upper(e.name)}({e.id}),")
    L.append("    }")
    L.append("")
    L.append("    /** backend ids (protocol §4.3 `status backend <id>`). */")
    L.append("    enum class Backend(")
    L.append("        val id: Int,")
    L.append("    ) {")
    for i, b in enumerate(backends):
        if i:
            L.append("")
        if b.note:
            L.append(f"        // {b.note}")
        L.append(f"        {upper(b.name)}({b.id}),")
    L.append("    }")
    L.append("}")
    L.append("")
    return "\n".join(L)


# ---------------------------------------------------------------------------
# main
# ---------------------------------------------------------------------------


def main() -> int:
    hooks, errs, backends = load()
    return emit_outputs(
        {
            OUT_KMOD: emit_kmod(hooks, errs, backends),
            OUT_PROTOCOL_RS: emit_rust(hooks, errs, backends),
            OUT_ZYGISK: emit_rust(hooks, errs, backends),
            OUT_LSP_KT: emit_kotlin(hooks, errs, backends),
        }
    )


if __name__ == "__main__":
    sys.exit(main())
