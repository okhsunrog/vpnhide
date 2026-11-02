# RFC 0004: Core attention state and native Android notifications

Decision: **Accepted for implementation** (2026-11-02). Code is not implemented by
this document. [ROADMAP](../../ROADMAP.md) tracks execution. This approval does
not choose a backend lifecycle, reboot, staging or fallback policy; RFC0003 remains
reopened. Notifications report accepted Core state rather than creating actions.

## Ownership

Core owns the bounded current attention set, update-check schedule, verification,
deduplication and recovery identity. An explicit private receiver in the APK is
only a wake signal. It fetches the current set over bounded authenticated RPC,
then renders/cancels Android NotificationManager notifications. No command body,
root path, remote URL or authoritative state arrives in the wake intent.

Use native Android channels: **Updates**, **ProtectionProblems** and
**ActionRequired**. Their Android settings and notification permission are the
controls; do not duplicate channel switches in application preferences. An absent
permission or blocked channel does not grant another delivery path, popup or forced
foreground activity. A force-stopped app is a legitimate unavailable destination;
do not bypass that state or promise guaranteed closed-app delivery.

Missing-Core bootstrap remains an APK/platform adapter with its own explicit
handoff. It must not grow into a second owner of runtime health or Core's update
schedule. No generic HTTP bridge becomes a root endpoint for notifications.

## State and delivery contract

Active attention has stable durable identifiers, typed reason, source generation,
currentness and optional action metadata. Bound the number and serialized size.
Updates require verified current release/compatibility evidence; a downloaded
hash alone is not publisher authentication. Update discovery never installs or
flashes Core automatically. Network failure retains a qualified previous result,
not a fabricated available/up-to-date claim.

A wake is advisory and may coalesce, arrive late or be lost. Re-query the complete
current set, not a delta inferred from intent payloads. Epoch/generation and durable
attention identity prevent late replies restoring withdrawn notifications. Repeated
wakes do not create duplicates. Withdrawal cancels the corresponding stable ID.
Application recreation, reinstall and service restart must reconcile native delivery
state with current Core state without replaying root actions.

Use boot/unlock grace and existing startup readiness before classifying persistent
attention. An unused backend or a normal pending initialization is not a protection
failure. Diagnostics evidence, configuration delivery and backend setup remain
separate. Incomplete/unknown evidence must not become success or an automatic
backend switch. Already installed process hooks retain their lifecycle meaning.

## Android adapter

The receiver performs bounded asynchronous work and authenticates/reconciles the
current RPC source. Core addresses the fixed application component/user through
the platform adapter; there is no arbitrary caller-selected broadcast target.
Normal background execution limits, denied permission, blocked channels, missing APK
and force-stop are explicit delivery outcomes. Do not start a persistent service,
restart zygote, disable another module or include stopped packages to force delivery.

Tapping a notification opens the app to review current state. Any privileged action
is still a typed Core command with its existing consent, operation identity and
unknown-outcome rules. A notification cannot authorize reboot or boot-image writes.

## Verification

Executable tests must cover current-set replacement, late/duplicate/lost wakes,
permission and channel denial, withdrawal/cancellation, restart/reinstall identity,
source skew, force-stop/unavailable delivery, boot grace and no inactive-backend
false alarm. Update verification and scheduling need offline clock/network tests,
with no production key or real manager effect. Kotlin adapters must not introduce
a parallel health classifier or update scheduler. Real Android background delivery
and permissions need later device acceptance.
