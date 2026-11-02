# RFC index

RFCs capture proposals and design decisions. Adding a document here does not
approve its implementation or prove that the described behavior exists.
Implementation progress and verification belong in the corresponding tracked plan.

| RFC | Status | Topic |
| --- | --- | --- |
| [0001](0001-automatic-gki-kmi-selection.md) | Accepted for implementation | Automatic GKI/KMI selection from the unified Core payload catalog |
| [0002](0002-autonomous-diagnostics.md) | Accepted for implementation | Autonomous native and Java diagnostics without relying on a third-party VPN |
| [0003](0003-conditional-zygisk-publication.md) | Reopened | Zygisk lifecycle review; ZIP dedup retained, activation policy paused |
| [0004](0004-core-attention-notifications.md) | Accepted for implementation | Core active attention,update scheduling and native Android notification channels |

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
