# What each diagnostic result means

- **Proven** — the required RAW signal, protected result and preservation controls
  were verified for this vector and recorded context.
- **Leak** — the protected worker could still observe the test signal. Other
  incomplete checks do not erase this evidence.
- **Unverified** — stimulus, observations, controls or attribution were incomplete.
  An empty enumeration or clean Java response alone cannot prove hiding.
- **Infrastructure error** — Core could not prepare, dispatch or complete the
  worker/fixture. Read its detail and use **Retry** after resolving the cause.
- **Cleanup incomplete** — resources could not be confirmed removed. Core retains
  this state and prevents unsafe reuse.

## Retained evidence

Earlier evidence can remain visible while offline, after config changes or after
an interrupted attempt. Its currentness and source context qualify what it proves.
Reconnect to Core or resolve an unknown operation before interpreting it as current.

There is no requirement to turn on a VPN or add VPN Hide to a split-tunnel list.
Dashboard availability is independent of those settings. A current worker result
also does not certify old game processes; restart the target app after Save.

See [What diagnostics checks](what-the-check-proves.md) and
[Saving and applying](saving-applying.md).
