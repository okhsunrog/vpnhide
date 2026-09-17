# Set up hiding

You configure hiding on the app you want to hide the VPN **from** — a bank, a
government service, a marketplace. The VPN app itself needs nothing here: it is
the thing being hidden. In a hurry? Jump straight to [the four roles](#the-four-roles).

> Example: to stop your banking app from seeing the VPN, give **the bank** the
> roles below. Your VPN client (WireGuard, for instance) is handled separately,
> in the hidden-VPN-apps list.

## Common situations

### The app detects the VPN

Give that app the **Java** and **Native** roles. Java covers the VPN signals
Android exposes to apps through its Java APIs; Native covers what the app can
see through the kernel or libc. Most detectors are caught by one of the two, so
enabling both is the reliable choice.

One exception: if your active Native backend is **Zygisk** and a banking or
payment app starts refusing to open once you enable Native for it, turn Native
off for that app and rely on Java — Zygisk runs inside the app's own process and
some of those apps detect it. That lowers coverage and doesn't guarantee the app
works; a kernel backend (kmod / KPM), where your device supports one, avoids the
problem.

### The app looks for an installed VPN client

Some apps don't probe the connection — they scan the list of installed packages
and refuse to run if they spot a known VPN app. Turn on the **Apps** role for
the app doing the scanning, and make sure your VPN client is in the hidden list
(**Settings → VPN app hiding**, or [open it now](vpnhide://hidden-apps)). VPN
Hide detects most VPN clients automatically; you can also hide one by hand.

### The app checks local ports

A few apps look for a VPN or proxy daemon listening on localhost. The **Ports**
role makes those ports look closed to that app. By default it hides every
localhost port; you can limit it to specific ranges in the per-app settings.

### You use a game accelerator

A game accelerator is itself a VPN, so VPN Hide treats it like any other — hide
its VPN from the apps that object, and hide the accelerator from package scans.
See **[Game accelerators](game-accelerators.md)** for the details, including why
the self-test can't run while the accelerator is on.

### When do my changes take effect?

After you tap **Save**: Java, the kernel backends and the ports rules apply
right away. Only Zygisk hooks need the target app to be force-stopped and
reopened, because they load into that app's process. Either way, an app that
already probed may have cached the old answer — restart it before you check
again.

### Everything is set, but the VPN is still detected

Open [Diagnostics](vpnhide://diagnostics) and run the checks. They test VPN
Hide's own hiding, not the specific behaviour of the other app, so a passing
result means the layers are working — the app may still refuse for its own
reasons (server-side checks, an installed-app scan you haven't covered, a cached
result).

If an app still won't cooperate, work through this in order:

- Check the roles are on the right app — the one you hide *from*, not the VPN
  client — and on the right profile.
- Tap **Save**, then force-stop and reopen the app so it probes again.
- Make sure the app's traffic goes where you expect (direct, or through the
  tunnel).
- If it scans for an installed VPN client, turn on **Apps** for it and hide the
  client.
- Still stuck? Collect a debug report from Diagnostics.

If networking broke or the app crashes after a change, undo your last change and
re-check **Ports** and **Native** one at a time — don't assume split tunneling is
the cause.

## The four roles

Set these on the app you are hiding from:

- **Java** — hides VPN signals exposed through Android's Java APIs
  (NetworkCapabilities, NetworkInfo, LinkProperties).
- **Native** — hides VPN signals visible through the kernel or libc, including
  the network-interface list. VPN Hide keeps one Native selection; only the
  active backend acts on it.
- **Apps** — gives the app a cleaned-up view of installed packages, so it can't
  see the VPN clients you've marked hidden.
- **Ports** — makes localhost VPN/proxy ports look closed to the app.

For Java, Native and Ports you can tap the settings icon next to the role to
choose individual hooks or port ranges for that app. A **\*** means the app has
custom per-hook settings.

Ready? [Open the Hiding tab](vpnhide://hiding) and pick the app to configure.

## Row, role chips and custom hooks

Tapping an unselected app row enables Java and Apps, plus Native and Ports when
their components are installed. Tapping a row with any role selected clears its
roles. Use the individual role chips to change only one layer. The row shortcut
also resets Java/Native hook selections to defaults; use chips if you want to
preserve custom choices. Changes still need **Save**.

Leave hook selection at its defaults unless troubleshooting a specific problem.
Disabling a hook reduces coverage; it is not a stronger stealth mode. Native
customizations for kernel backends and Zygisk are stored separately, so review them
when changing backend. Ports ranges are a separate policy.

## Automatic and manual package hiding

Open [VPN app hiding](vpnhide://hidden-apps). This chooses **what packages to hide**;
the observer still needs the **Apps** role and working LSPosed hooks.

- **Auto:** detected from an Android VpnService declaration. This is package
  discovery, not evidence that a VPN is connected.
- **Name matching:** an optional, less reliable heuristic for names containing VPN;
  it is off by default and can produce false matches.
- **Manual:** a package you explicitly chose to hide.
- **Excluded:** a package exempted from automatic hiding after a false match.
  Unchecking a package removes manual hiding and, for an automatic match, adds an
  exclusion. Checking it again removes the exclusion.

Changing the hidden-package list affects all Apps observers. Test the target after
applying and restarting it. Use exclusions instead of disabling automatic discovery
for everyone just to correct one false match.
