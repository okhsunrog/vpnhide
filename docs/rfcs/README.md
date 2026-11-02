# RFC index

RFCs capture proposals and design decisions. Adding a document here does not
approve its implementation or prove that the described behavior exists.
Implementation progress and verification belong in the corresponding tracked plan.

| RFC | Status | Topic |
| --- | --- | --- |
| [0001](0001-automatic-gki-kmi-selection.md) | Proposed | Automatic GKI/KMI selection from the unified Core payload catalog |
| [0002](0002-autonomous-diagnostics.md) | Draft | Autonomous native and Java diagnostics without relying on a third-party VPN |

Statuses:

- **Draft** — being developed and discussed.
- **Proposed** — ready for review; not accepted for implementation.
- **Accepted** — an explicitly approved design, not a claim of completed code.
- **Implemented** — linked implementation and verification establish completion.
- **Superseded** — replaced by another linked RFC.

Use the next sequential number and a short descriptive filename. Preserve the
original proposal; record substantive review comments separately so the author's
text and later decisions remain distinguishable. Internal “decision” sections do
not override the document's status or authorize experiments by themselves.

The initial imports preserve the submitted text. RFC 0002 changes only its proposed
repository-path metadata to match this directory. RFC 0001's kernel-loader preflight
and RFC 0002's diagnostic fixtures remain proposals requiring separate validation;
creating this index does not enable them.
