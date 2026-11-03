# Shared snapshot codec and validation — 3 November 2026

Engineering migration after `e0bf7ec5deb4be5197257a2e404e53021188ae02`.
Exact source hashes, final logs, JVM reports and real FFI ELF headers are retained
under `vpnhide-build-evidence/shared-probe-frames/qualification.json` in the execution
workspace. This is not a release or Android device acceptance.

Root/debug framing, prefixes, output bound, required root sections and validation
now have one portable Rust owner in `core-rpc::probe_frames`. The real Core
collector reuses its prefixes/output cap; generated local FFI serves the APK's
RootSnapshotCache and DebugShellSnapshot adapters. No RPC, privileged command,
arbitrary root path or new authority was added by the codec API.

The retired Kotlin parser/policy structs and duplicated frame constants are gone.
Root discards incomplete/mismatched sections and validates the full required set;
forensic debug keeps partial bodies and separate bound/parent-probe failures.
LF/CRLF/bare-CR, inline END, duplicate-key order, whitespace and signed timing
behavior are preserved. Input, name, section and timing counts are bounded.
Kotlin retains StartupTrace callbacks, source joins, error presentation and the
human-readable partial marker. PackageManager observation adapters are unchanged.

The producer test no longer reads or parses Kotlin source constants. An independent
required-key JSON fixture pins the contract, and actual emitted output must satisfy
it. Three Kotlin source-text guards and their filesystem-source helper are removed.
Actual Core tests cover UID/identity output, production paths, counter-section
scope, inventory arguments and redaction. Rust adversarial/golden tests own the
codec; retained JVM cases now exercise the actual local FFI adapters. A differential
JVM case compares whitespace behavior over valid BMP scalars to Char.isWhitespace,
including NUL/Unicode transport through the generated binding.

Final gates:558 Rust tests across the three changed packages,588 JVM tests, strict
host/all-target Clippy including oracle/tooling and both Android ARM library targets,
fmt, ktlint, detekt, CPD, lintDebug, scaffold/no-su and actual FFI16KiB LOAD/congruence.
The integrated capacity test waits for the typed native publication after a durable
terminal receipt; those are separate publications. It still requires capacity
refusal and unchanged backend bytes and never retries the mutation. The earlier
inherited-file-lock failure was independently reproduced and fixed in `e0bf7ec5`.
Neither transient publication timing nor a failed test is counted as a successful gate.

RPC31/journal14/diagnostic9/debug schema11 and valid wire text are unchanged.
Snapshot reads admit no autonomous suite and depend on no foreign VPN or split
tunnel. New backend policy remains paused. Capture/export/agent orchestration,
notifications and remaining diagnostic stimuli remain code work in the roadmap.
A genuine secondary32-bit Android worker remains an explicit implementation gap;
ARMv7 compile/ELF success is not runtime coverage. No phone, provider setting,
production key, release or boot write was used.
