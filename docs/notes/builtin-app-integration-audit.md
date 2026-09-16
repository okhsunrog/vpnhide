# Built-in backend app integration audit

Scope: application consumers of native-backend identity, based on main
`eec0d95f` (2026-09-16). Kernel hook implementations are outside this audit.

Confirmed gaps addressed:

- Picker availability and inactive-backend display omitted the built-in companion.
  Whole-row selection consequently persisted `native: false`.
- Statistics hard-coded KMOD for the shared kernel channel: built-in counters were
  mislabelled, and an active backend without counters disappeared.
- Hook reports associated the shared status with both kernel identities and
  expected the optional built-in filesystem hook even when not installed.
- Debug capture omitted companion metadata, activation boot status, binary hash
  and staged-update evidence.
- Active-backend conflict classification omitted built-in alongside KPM/Zygisk.
- Reset omitted the companion blocker and its state directory. Reboot guidance
  incorrectly implied that removing the companion removes compiled-in hooks.

Existing built-in handling was traced in ConfigChannels activation priority,
RootSnapshotCache collection, backend detection and display, native hook-family
selection, filesystem settings, diagnostic ownership/missing-hook calculations,
and module integrity/version reporting. These paths already include built-in;
this source audit is not hardware validation of every state or ROM.

Regression tests cover built-in-only row selection, inactive companion display,
statistics before/after the first interception, optional-hook expectations and
reset blocking. Existing shell syntax checks cover the expanded debug collector.
Full reset must not be exercised against a working test phone.
