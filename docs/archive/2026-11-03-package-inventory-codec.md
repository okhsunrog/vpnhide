# Shared PackageManager package/UID codec — 3 November 2026

Engineering ownership slice after KPM lock correction
`fea3e9837d760318c6313c57aadde8d57d5f37a7`. Source hashes, logs, JVM reports
and actual FFI ELF qualification are retained under
`vpnhide-build-evidence/package-inventory-codec/qualification.json` in the execution
workspace. This is not a release or device acceptance.

`native-policy::package_inventory` now owns package-row parsing for both plain and
APK-path forms, UID normalization/union, bounded input and per-user scan framing.
Core's PackageUidMap and APK typed local FFI consume that grammar. The Core producer
uses its marker constants. Kotlin no longer owns parsePackageUidLine, its text UID
union, mutable scan builder or status-marker parser. Android profile display/type
metadata, PackageManager acquisition/backstop, localized messages and observation
cache wiring remain adapters; this does not complete their reconciliation scheduling.

Nonnegative signed32 Android UIDs include platform AIDs and never wrap into Kotlin
Int. APK paths can contain spaces and equals signs. Unframed comma-separated UIDs
are partitioned by actual user rather than copied into every inferred profile.
A matching successful empty profile is valid. Readable packages survive failed
and truncated scans with incomplete status. Unmatched END markers and later
truncated retries cannot reuse a previous successful scan status. Local input
bounds fail explicitly; display markers/backstop never replace fresh Core admission.

Rust owns adversarial grammar/control tests. Three trivial Kotlin text-parser
cases were retired after moving their coverage; retained and added JVM cases now
exercise generated FFI and the actual package-inventory adapters. The independent
marker fixtures are literals rather than generated parser constants.

Final integrated verification:639 Rust tests,587 JVM tests, zero failures or skips;
strict host/all-target Clippy including tooling and the fixture oracle, both Android
ARM library Clippy, fmt, ktlint, detekt, CPD, lintDebug, scaffold/no-su and actual
FFI16KiB LOAD alignment/congruence. The earlier failed KPM delivery gate was diagnosed,
reproduced independently and fixed in the preceding commit, not accepted as a pass.

RPC31, private journal14, diagnostic9, debug schema11 and valid snapshot text are
unchanged. Save/Cancel, uncertain Save recovery and final cloud startup-contention
regressions remain intact. A new explicit current-contract/roadmap section records
that the genuine secondary32-bit Android worker launcher is **unimplemented**;
ordinary Service ABI, ARMv7 compile/ELF and an ARM64 worker do not demonstrate it.
No loader proof was relaxed. A safe launcher is code work before later runtime
acceptance. No phone/provider operation, production key, boot write or release was
used; backend selection/publication policy remains paused. Remaining capture/export,
agent orchestration, notifications and diagnostic stimuli remain separate code work.
