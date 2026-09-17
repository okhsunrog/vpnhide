# Reading the statistics

**Interception counters** show calls handled by the installed hooks, grouped by
UID/app and hook. Tap an app for its breakdown. A normal network API call can hit
a hook too: a count is not proof that the app deliberately tried to detect a VPN.

## Observe activity during a test

1. Tap **Start capture** to take a counter baseline.
2. Switch to the target app and reproduce the behavior.
3. Return and tap **Stop** to keep the displayed differences for review.

During capture, VPN Hide periodically reads cumulative counters and shows the
increase since the baseline. This is **not a trace of individual calls**: it does
not provide every call's exact time, order or arguments. Other apps may appear too.

Stopped results remain in the current app process until cleared or replaced by a
new capture; they are not a durable recording after a force-stop or reboot. For
logs to send with a bug report, use [debug recording](collect-report.md).

## Backend differences and missing data

- Kernel and Java counters accumulate while their runtime remains alive. A reboot
  resets them; a backend reset can also restart the capture baseline.
- **Zygisk:** native counters are unavailable; LSPosed still supplies Java and Apps counters.
- **KPM:** current components read native counters in pages. If an older/truncated
  reply is encountered, incomplete native totals are withheld rather than shown as
  complete. Follow the screen's message and check component versions.
- An empty or unavailable result does not prove the app made no checks. Check for
  read errors, unsupported counters and detection paths outside the installed hooks.

Counters measure activity, not complete protection. Use the
[self-test](what-the-check-proves.md) for its measured vectors and reproduce the
actual behavior in the target app.
