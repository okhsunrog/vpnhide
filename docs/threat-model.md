# Threat model

vpnhide reduces the local evidence of a VPN available to selected Android apps.
It does not provide anonymity, hide root, or guarantee that an app or its server
will accept the device. This document defines the attacker, trusted components
and limits of that claim. The per-interface coverage contract remains in
[detection-vectors.md](detection-vectors.md); storage permissions remain in
[storage.md](storage.md) and [state.md](state.md).

## Protected properties

For an app selected for the relevant roles, vpnhide aims to:

- Filter supported local VPN signals from the native and Android framework
  interfaces it intercepts, while keeping ordinary app networking usable.
- Filter configured packages from supported package-manager queries, and block
  configured localhost port probes when the corresponding roles are active.
- Keep privileged policy, control operations and credentials inaccessible to
  ordinary apps under the documented Android permission and SELinux assumptions.
- Report the scope and freshness of diagnostic evidence, including unknown,
  interrupted and partially applied states, without presenting them as success.

These are scoped properties, not a promise of complete observational equivalence
to a phone without a VPN. Every hiding claim depends on the selected backend,
installed hooks, enabled options, target identity and actual device behavior.

## Attacker and capabilities

The primary attacker is an **unprivileged target app** on the user's device,
including native libraries and SDKs running with that app's privileges. It can:

- Use public Java APIs, Binder requests, native code and direct syscalls available
  to its UID and SELinux domain.
- Enumerate accessible interfaces, routes, packages and local endpoints; compare
  synchronous results with callbacks, cached values and observations over time.
- Inspect its own process and attempt to identify injected code or modified
  functions. This matters especially for the in-process Zygisk backend.
- Contact external servers and combine local observations with remote evidence.
  Remote evidence remains available even when local filtering works.

The app is not assumed to cooperate or restrict itself to the built-in test
suite. The known vector matrix is not an exhaustive list of everything an
adversarial app could try.

The model excludes an attacker who controls the kernel, `system_server`, the
root manager, the hooking framework or an authorized root process. It also does
not promise hiding from privileged OEM/system apps, a compromised operating
system, or a physical/boot-chain attacker. Their access can exceed the ordinary
app boundary, invalidate policy assumptions or disable the hooks entirely.

## Trusted components and boundaries

The user controls the device, approves root access and installs trusted builds
of vpnhide and its dependencies. Trust includes the kernel/ROM, root manager,
selected native backend and its loader, LSPosed when used, the picker app,
activator and privileged helper. Compromising any of these can undermine the
result; vpnhide does not isolate them from one another as hostile principals.

| Boundary | Enforcement and limits |
| --- | --- |
| Target app → kernel observations | kmod, built-in and KPM filter supported kernel paths for target UIDs. Direct syscalls do not bypass those particular hooks; unhooked paths remain outside their coverage. |
| Target app → in-process libc | Zygisk filters hooked libc calls. Raw syscalls, unhooked entry points and process inspection remain relevant bypass/detection paths. |
| Target app → framework services | LSPosed hooks in `system_server` sanitize supported Binder results and callbacks using caller identity. Coverage depends on hooks attaching on the actual Android/ROM implementation. |
| Target app → local ports | The Ports role uses configured firewall rules. It does not hide every possible endpoint or remote service and does not replace the other roles. |
| App configuration → privileged runtime | The config coordinator owns app writes; the activator derives backend state. Helper receipts and session fencing track privileged work, including late launches and uncertain outcomes. They are not protection from a malicious root caller. |
| Ordinary app → stored policy/secrets | Documented ownership, modes and SELinux labels separate ordinary apps from canonical policy and privileged control data. Root remains trusted and can read or change them. |

See [config-coordinator.md](config-coordinator.md),
[root-mutation-transport.md](root-mutation-transport.md) and
[protocol.md](protocol.md) for those contracts. Installing a module or saving a
configuration alone does not establish that runtime activation succeeded.

Target selection is not always a package-private boundary. Native kernel
filtering uses UIDs. Java hook masks are unioned by appId, including packages
sharing that appId and its copies across profiles. The picker applies one
package selection to its discovered profile copies; it does not offer separate
personal/work roles for the same package. Processes with different identities
must not be assumed covered merely because they belong to the same app workflow.
See [state.md](state.md) and [work profiles](help/en/work-profiles.md).

## Operating assumptions and residual risks

### Coverage and platform variation

Use one active Native backend and the other roles required for the target's
probes. Kernel-level filtering removes the libc-only bypass for the hooked
vectors; it is not a universal anti-detection guarantee. Optional hooks may need
an app restart or device reboot. Unsupported interfaces, renamed/unrecognized
interfaces, missing hook attachment and backend capacity limits can leave gaps.

SELinux may deny some probes on a particular device. Such a denial does not
prove that vpnhide hid the VPN, and a different ROM, privileged domain or
permissive policy can expose the same path. Do not generalize one device's
SELinux behavior into coverage. Consult the current
[coverage matrix and known gaps](detection-vectors.md) and [limits](limits.md).

### Routing and remote detection

vpnhide is not a VPN client or a kill switch. It does not promise to keep every
target connection inside a tunnel, prevent DNS/IP leaks, or alter the remote
server's view of the source IP, account history, timing or protocol behavior.

Java concealment can substitute a physical cover network for the VPN network.
An app explicitly binding to that returned network may use the physical path
if Android permits the UID to use it and the VPN policy allows bypass. Returning
a physical `Network` does not itself establish whether a connection bypasses the
VPN or is blocked. Android documents this distinction in
[`VpnService.Builder.allowBypass()`](https://developer.android.com/reference/android/net/VpnService.Builder#allowBypass()).
Therefore, hiding success must not be interpreted as proof of tunnel retention.
Split tunneling is the usual setup when the target should use the physical
connection; it intentionally exposes that connection's public IP to the service.
A VPN-preserving concealment mode remains a separate design direction in the
[roadmap](notes/ROADMAP.md). VPN routing and lockdown policies require their own
verification; local concealment does not override or certify them. The network
substitution is implemented in
[HookEntry.kt](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/hook/HookEntry.kt),
with cover selection in
[VisibleNetworkResolver.kt](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/hook/VisibleNetworkResolver.kt).

### Failures, stale state and availability

This is not a fail-closed network access control system. For example, an LSPosed
system-server policy cache read error retains the last successful policy (or an empty initial policy) and
reports the error; it does not stop all target traffic until recovery. Config
persistence, activation and observed runtime state are separate phases. A failed
or uncertain phase is not proof that the requested protection is in force.

An app may retain observations collected before activation. UID/profile changes,
VPN transitions, target restarts and OS updates can change applicability. Hooks
can also affect compatibility or stability, especially in kernel and framework
paths. The threat model does not promise availability under arbitrary hostile
probe workloads or immunity to hook implementation bugs.

## Credentials, developer access and exported evidence

The APatch/FolkPatch SuperKey authorizes KernelPatch control. The app does not
offer a session-only key mode. Saving the key in Settings persists it in the
root-owned `0600` file
`/data/adb/vpnhide/superkey` until cleared; no key is saved by default. The key is
separate from canonical config exports. The APatch activator tries the saved key
and the trusted `su` token; a working trusted token makes saving a key
unnecessary. The boot-readable copy is not an app-only Keystore secret. Root
compromise defeats that boundary. Backend-specific
keyless operation and boot activation requirements are described in
[storage.md](storage.md). The Save/Clear path lives in
[SettingsScreen.kt](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/settings/SettingsScreen.kt);
authentication candidates are selected in
[kpm.rs](../crates/activator/src/kpm.rs).

The optional developer HTTP bridge is off by default, binds to `127.0.0.1`, and
requires a generated bearer token. **Loopback is not app identity**: the token is
the authorization boundary for local clients. The implementation stores it in
app-private files and deliberately logs it for the host tooling's logcat
fallback. Anyone who obtains that token and can reach the bridge must be treated
as an
authorized controller, including for configuration mutations. Forwarding the
bridge can extend its reach beyond local clients; it still requires the token.
Request size limits and a receive deadline bound incoming request
parsing; that deadline does not bound execution of an accepted operation. These
limits are not a general denial-of-service guarantee. See
[AgentControlBridge.kt](../lsposed/app/src/main/kotlin/dev/okhsunrog/vpnhide/AgentControlBridge.kt).

Debug archives are intentionally useful evidence, not anonymous reports. Depending
on options they contain configuration, network addresses/routes, device and
module details, logs, app inventory or kernel images. Redaction removes specific
fields; it does not establish that every secret or identifying value is absent
from arbitrary logs or images. The user guide asks users to choose export categories and share reports through
an agreed support channel rather than inspect a large JSON document manually.
Limiting recipients does not make an archive anonymous.
See [debug-bundle.md](debug-bundle.md) for the exact fields and redaction rules.

## What diagnostics establish

The self-test compares the picker process's app-side probes with root-observed
state. A meaningful run requires an active VPN and the picker itself routed
through it. A VPN-off or self-excluded state is not a successful hiding test.

The checks execute real detection probes, not just hook-installation checks.
A successful hiding measurement establishes that the tested signal was hidden
under the measured conditions; root-differential probes compare app-visible
results with root-observed state. A completed run establishes evidence for the
measured vectors, process and conditions. It does not execute inside every target app, certify every callback
sequence, prove that all traffic stayed in the tunnel, or establish acceptance by
a remote service. Different target settings, identities and APIs can give a
different result. Root-denied, unmeasured and uncovered cases must retain their
own meanings rather than becoming a hiding success.

UI and exports use a shared diagnostic snapshot. Its revision identifies
accepted application knowledge, not simultaneous sampling of the whole device.
A retained measurement can become changed or unverified after routing/config
changes or invalidation; its run ID and observation provenance remain useful
history. See [diagnostics.md](diagnostics.md),
[observation-coordinator.md](observation-coordinator.md) and
[what the self-test proves](help/en/what-the-check-proves.md).

The localized [security and privacy guide](help/en/security-and-privacy.md)
presents these limits in the app. Keep its claims consistent with the verified
implementation when updating this model.

## Maintaining this model

Update this document when attacker capabilities, trust boundaries, credential
exposure or the meaning of a protection claim changes. Update
[detection-vectors.md](detection-vectors.md) for hook coverage, not a duplicate
matrix here. Validate changes at the relevant boundary: pure/unit tests for
parsers and state transitions, native/kernel tests for their execution paths,
and device tests for ROM attachment, SELinux, routing and target behavior.
A green build or one passing detector is evidence within that scope, not proof
that all possible VPN detection has been defeated.
