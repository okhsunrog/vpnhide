# Historical evidence and proposals

These dated documents preserve implementation history, audits, experiments and
unapproved ideas. Their old status, commands, UI and test counts are historical.
They are not the current work index or authority to implement a reopened design.

- [KPM NULL-ifconf qualification](2026-11-03-kpm-ifconf-qualification.md)
- [Controlled interface-path lookup qualification](2026-11-03-native-filesystem-fixture-qualification.md)
- [GKI modpost qualification experiment](2026-11-03-gki-modpost-qualification.md)
- [Current roadmap](../../ROADMAP.md)
- [Current contracts](../README.md)
- [RFC decisions](../rfcs/README.md)
- [Implementation history](2026-11-02-unified-core-implementation.md)
- [Previous roadmap and retained ideas](2026-11-02-ROADMAP.md)
- [Kotlin/Core audit](2026-11-02-kotlin-core-architecture-audit.md)
- [Host tooling record](2026-11-02-host-tooling-audit.md)
- [Prior state-transition contract](2026-11-02-app-state-transitions.md)
- [State-transition history](2026-11-02-app-state-transitions-history.md)
- [Old observations/VPN listener contract](2026-11-02-core-2.0-observations.md)
- [Activator cutover history](2026-11-02-core-2.0-activator-cutover.md)
- [Hardening plan](2026-11-02-core-v2-hardening-plan.md)
- [Earlier state design](2026-11-02-app-state-design.md)
- [Device validation at the time](2026-11-02-vpn-poll-device-validation.md)
- [UI review](2026-11-02-ui-state-presentation-review.md)
- [Diagnostics analysis](2026-11-02-diagnostics-state-analysis.md)
- [Builtin integration audit](2026-11-02-builtin-app-integration-audit.md)
- [Setup wizard proposal](2026-11-02-setup-wizard-design.md)
- [Release-candidate proposal](2026-11-02-release-candidates.md)

Archive dates identify the documentation reorganization. Original source pins and
assessment dates remain in the documents; an archive date does not claim a fresh
device test. Code links use retained repository snapshots where available.

- [Diagnostic worker ABI launcher source review](2026-11-03-diagnostic-worker-abi-launch.md): instrumentation restarts the target package; not a mixed-ABI implementation or device proof.
- [Suspend-aware diagnostic deadline qualification](2026-11-03-diagnostic-deadline-qualification.md): Core boot-time authority, retained evidence and inherited framework expiration.
- [KPM RAW lifecycle source review](2026-11-03-kpm-raw-lifecycle-review.md): disabled quarantined prototype; unresolved runtime control/unload lifetime, not device acceptance.
- [Clean `21cd31ee` bundle and unsigned APK](2026-11-03-clean-bundle-21cd31ee.md): seven fresh qualified KMI payloads, both Core/Zygisk ABIs, full assembly, R8 and scoped startup regression; no production signing or device acceptance.
- [Controlled interface-directory qualification](2026-11-03-native-directory-fixture-qualification.md): six libc/direct-getdents64 sysfs/proc-sys controls, real RAW/protected QEMU and retained evidence; stat/access/open and Android acceptance remain separate.
- [Historical mixed bundle guide](2026-11-03-debug-bundle-legacy-reference.md): source-pinned old report/gate/verdict instructions for historical bundles; the current guide uses retained Core evidence.
