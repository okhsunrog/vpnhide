# Set up hiding

You configure hiding on the app you want to hide the VPN **from** — a bank, a
government service, a marketplace. The VPN app itself needs nothing here: it is
the thing being hidden.

> Example: to stop your banking app from seeing the VPN, give **the bank** the
> roles below. Your VPN client (WireGuard, for instance) is handled separately,
> in the hidden-VPN-apps list.

## Common situations

### The app detects the VPN

Give that app the **Java** and **Native** roles. Java covers the VPN signals
Android exposes to apps through its Java APIs; Native covers what the app can
see through the kernel or libc. Most detectors are caught by one of the two, so
enabling both is the reliable choice.

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

A game accelerator (UU, Xunyou, Tencent and the like) is itself a VPN: on
Android it builds a VpnService tunnel and routes the games you pick through it.
The system therefore sees an active VPN and an installed VPN app, and other apps
can react to that. If a bank or another app refuses to run "because a VPN is on"
while your accelerator is active, hide the VPN from *that* app with **Java +
Native**, and add the accelerator to the hidden VPN-apps list so package scans
don't spot it. Because the accelerator only routes the games you select, the app
you are hiding from is off the tunnel anyway.

This is about hiding the accelerator's VPN from *other* apps. Hiding it from the
game you are accelerating, to get past that game's own checks, is a different
matter and not something VPN Hide can promise.

Accelerators usually route a fixed game list, not arbitrary apps, so they often
won't let you add VPN Hide to their tunnel. That is why the Overview self-test
may report that it can't run while the accelerator is on — that is expected, not
a failure. Judge hiding by whether your target app works.

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
result). See **Fix a problem** for the next steps.

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
