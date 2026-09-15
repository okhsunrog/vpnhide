# Reading the statistics

The **Interception counters** view shows how often the apps you selected actually
hit an installed hook — the runtime events the active backends report, counted
**cumulatively since boot**. It's a window into what your target apps probe, not
a scoreboard.

## What you see

- **Counters stay empty** until a selected app calls a hook. "No runtime
  statistics yet" just means nothing you targeted has probed since boot.
- **Apps probing VPN** lists the apps that hit hooks; tap one for its full
  per-hook breakdown (which detection paths it used).

## Capture session — watch one app live

To see exactly which VPN checks a specific app runs:

1. Tap **Start capture**.
2. Switch to the app you want to test and use it for a bit.
3. Every VPN check it makes shows up in the list as it happens.

Tap **Stop** when done; the results stay until you start a new capture or clear
them. This is the quickest way to understand *how* a particular app looks for a
VPN.

## Backend differences

- **Zygisk** — native counters aren't available. Java and Apps counters are still
  reported by LSPosed.
- **KPM** — it has more counters than its control channel can return at once, so
  partial native totals are hidden; Java and Apps counters still show.

## What not to read into it

A high count doesn't mean an app is "more suspicious," and a **zero** count
doesn't prove an app never checks — it may use a detection path this backend
doesn't cover, or it cached an earlier result. Counters show activity on the
hooks that are installed; they're not proof that hiding is complete. For that,
use the self-test — see [What the self-test checks](what-the-check-proves.md).
