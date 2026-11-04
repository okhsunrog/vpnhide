# RFC index

RFCs capture proposals and design decisions. Adding a document here does not
approve its implementation or prove that the described behavior exists.
Implementation progress and verification belong in the corresponding tracked plan.

| RFC | Status | Topic |
| --- | --- | --- |
| [0001](0001-automatic-gki-kmi-selection.md) | Accepted for implementation | Automatic GKI/KMI selection from the unified Core payload catalog |
| [0002](0002-autonomous-diagnostics.md) | Accepted for implementation | Autonomous native and Java diagnostics without relying on a third-party VPN |
| [0003](0003-conditional-zygisk-publication.md) | Policy superseded by0005 | Zygisk packaging/projection constraints retained; approved continuity decision in0005 |
| [0004](0004-core-attention-notifications.md) | Accepted for implementation | Core active attention,update scheduling and native Android notification channels |
| [0005](0005-unified-backend-lifecycle-policy.md) | Accepted for implementation and verified main publication | Working-backend continuity, fresh Builtin/KMOD preference, explicit Zygisk and no inter-family automatic fallback |

Statuses:

- **Draft** — being developed and discussed.
- **Proposed** — ready for review; not accepted for implementation.
- **Reopened** — a previously accepted design is under renewed review; its implementation permission may be paused.
- **Accepted** — an explicitly approved design, not a claim of completed code.
- **Implemented** — linked implementation and verification establish completion.
- **Superseded** — replaced by another linked RFC.

Use the next sequential number and a short descriptive filename. Preserve the
original proposal; record substantive review comments separately so the author's
text and later decisions remain distinguishable. Internal “decision” sections do
not override the document's status or authorize experiments by themselves.

The initial imports preserve the submitted text and original status metadata.
Later maintainer approval authorizes implementing RFC0001 and RFC0002 in this
cycle; their separate kernel/device evidence gates remain mandatory. Neither
acceptance nor this index enables experimental production admission. RFC0003's
newer pause overrides its earlier implementation approval.

RFC0005 proposes a coherent replacement for RFC0003's strict default. Publishing
it does not resolve the reopened decision or lift the pause on new activation
policy. The agreed requirements and remaining proposed details are distinguished
inside the document.

The 3 November maintainer amendment excludes Zygisk from Auto, automatic
fallback and standby publication. Only explicit informed “Use Zygisk” selection
permits preparation; prior consent/use never licenses later automatic reentry.
There is no offer for sufficient working kernel protection. RFC0005 and RFC0003
record this requirement and legacy retirement limits. The flow is not implemented;
the broader policy pause remains in force.
