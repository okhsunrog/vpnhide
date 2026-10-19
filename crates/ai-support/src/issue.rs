//! Deterministic GitHub issue draft.
//!
//! Built from the snapshot the client already sent. No model, no GitHub call,
//! and no sentence that turns an unmeasured run into "hiding works".

use crate::snapshot::{CheckOutcome, DiagnosticGate, DiagnosticSnapshot, NativeBackendId};

/// Title and body a later GitHub App would file. The body is the whole draft:
/// archive bytes never go in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueDraft {
    pub title: String,
    pub body: String,
}

const MAX_TITLE_CHARS: usize = 120;

pub fn draft_issue(
    version: &str,
    snapshot: &DiagnosticSnapshot,
    question: &str,
    archive_consent: bool,
) -> IssueDraft {
    IssueDraft {
        title: title(version, snapshot),
        body: body(version, snapshot, question, archive_consent),
    }
}

fn title(version: &str, snapshot: &DiagnosticSnapshot) -> String {
    let subject = if let Some(id) = first_owned_leak(snapshot) {
        format!("owned leak {id}")
    } else if snapshot.measurement_is_meaningful() {
        "measured run".to_string()
    } else {
        let gate = snapshot
            .gate
            .map(DiagnosticGate::as_str)
            .unwrap_or("no gate");
        format!("not measured ({gate})")
    };
    let raw = format!("[support] {version} {subject}");
    if raw.chars().count() <= MAX_TITLE_CHARS {
        raw
    } else {
        let mut end = MAX_TITLE_CHARS;
        while end > 0 && !raw.is_char_boundary(end) {
            end -= 1;
        }
        raw[..end].to_string()
    }
}

fn body(
    version: &str,
    snapshot: &DiagnosticSnapshot,
    question: &str,
    archive_consent: bool,
) -> String {
    let mut out = String::new();
    out.push_str("## Report\n\n");
    push_facts(&mut out, version, snapshot);
    out.push('\n');
    push_reading(&mut out, snapshot);
    out.push_str("\n## Question\n\n");
    for line in question.lines() {
        out.push_str("> ");
        out.push_str(line);
        out.push('\n');
    }
    out.push_str("\n## Archive\n\n");
    let consent = if archive_consent { "yes" } else { "no" };
    out.push_str(&format!("Full diagnostic archive consent: {consent}.\n"));
    out.push_str(
        "This sketch does not store the archive. Logcat, dmesg, and raw shell sections are not part of this draft.\n",
    );
    out.push_str("\n## Filing\n\n");
    out.push_str("GitHub App is not configured. This draft was not filed.\n");
    out
}

fn push_facts(out: &mut String, version: &str, snapshot: &DiagnosticSnapshot) {
    out.push_str(&format!("- Checkout version: `{version}`\n"));
    out.push_str(&format!(
        "- Snapshot app version: `{}`\n",
        snapshot.app.version
    ));
    if version != snapshot.app.version {
        out.push_str("- The checkout version and the snapshot app version differ.\n");
    }
    out.push_str(&format!(
        "- Device: {} {}, Android {} (sdk {})\n",
        snapshot.device.manufacturer,
        snapshot.device.model,
        snapshot.device.android_release,
        snapshot.device.sdk
    ));
    let gate = snapshot.gate.map(DiagnosticGate::as_str).unwrap_or("none");
    out.push_str(&format!("- Gate: `{gate}`\n"));
    out.push_str(&format!(
        "- Native verdict: {}\n",
        option_label(snapshot.native_verdict.map(|verdict| verdict.as_str()))
    ));
    out.push_str(&format!(
        "- Java verdict: {}\n",
        option_label(snapshot.java_verdict.map(|verdict| verdict.as_str()))
    ));
    out.push_str(&format!(
        "- Active backend: {}\n",
        backend_label(&snapshot.active_backend.id, &snapshot.active_backend.state)
    ));
    out.push_str(&format!(
        "- Root shell uid: {}, runtime_checkable: {}\n",
        snapshot
            .root_shell
            .uid
            .map(|uid| uid.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        snapshot.root_shell.runtime_checkable
    ));
    out.push_str(&format!(
        "- self_needs_restart: {}\n",
        snapshot.self_needs_restart
    ));
    if let Some(summary) = &snapshot.diagnostics {
        out.push_str(&format!(
            "- Applicability: `{}`\n",
            summary.applicability.as_str()
        ));
        out.push_str(&format!("- currentSuccess: {}\n", summary.current_success));
        if let Some(evidence) = &summary.evidence {
            out.push_str(&format!(
                "- Evidence conclusion: `{}`\n",
                evidence.conclusion.as_str()
            ));
        }
    }
}

fn push_reading(out: &mut String, snapshot: &DiagnosticSnapshot) {
    if snapshot.allows_positive_claim() {
        out.push_str(
            "The snapshot reports currentSuccess for a routed measurement that still applies and records no owned leak. That describes this measurement only.\n",
        );
    } else if snapshot.measurement_is_meaningful() {
        out.push_str(
            "The self-test ran with gate `routed`. Verdicts below are that measurement, not a claim that hiding holds for every app.\n",
        );
    } else {
        let gate = snapshot.gate.map(DiagnosticGate::as_str).unwrap_or("none");
        out.push_str(&format!(
            "Gate is `{gate}`. Only `routed` is a measurement, so this snapshot does not show whether hiding worked.\n"
        ));
    }
    if let Some(id) = first_owned_leak(snapshot) {
        out.push_str(&format!(
            "Check `{id}` leaked on a vector the active backend owns.\n"
        ));
        if let Some(hooks) = missing_hooks_for(snapshot, id) {
            if !hooks.is_empty() {
                out.push_str(&format!(
                    "The active backend reported these hooks as not installed: {}.\n",
                    hooks
                        .iter()
                        .map(|hook| format!("`{hook}`"))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }
    if !snapshot.root_shell.runtime_checkable {
        out.push_str(
            "runtime_checkable is false, so an inactive module in this snapshot is not a confirmed inactive module.\n",
        );
    }
}

fn first_owned_leak(snapshot: &DiagnosticSnapshot) -> Option<&str> {
    let report = snapshot.report.as_ref()?;
    report
        .native
        .checks
        .iter()
        .chain(report.java.checks.iter())
        .find(|check| check.owned && matches!(check.outcome, CheckOutcome::Leak))
        .map(|check| check.id.as_str())
}

fn missing_hooks_for<'a>(snapshot: &'a DiagnosticSnapshot, id: &str) -> Option<&'a [String]> {
    let report = snapshot.report.as_ref()?;
    report
        .native
        .checks
        .iter()
        .chain(report.java.checks.iter())
        .find(|check| check.id == id)
        .map(|check| check.missing_hooks.as_slice())
}

fn option_label(value: Option<&str>) -> String {
    value
        .map(|value| format!("`{value}`"))
        .unwrap_or_else(|| "null".to_string())
}

fn backend_label(id: &Option<NativeBackendId>, state: &crate::snapshot::ModulePresence) -> String {
    use crate::snapshot::ModulePresence;
    let name = id.map(NativeBackendId::as_str).unwrap_or("none");
    let presence = match state {
        ModulePresence::NotInstalled => "not installed",
        ModulePresence::Installed { active: true, .. } => "active",
        ModulePresence::Installed { active: false, .. } => "installed, not active",
    };
    format!("`{name}` ({presence})")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshot::{
        DiagnosticEligibility, DiagnosticGate, EvidenceConclusion, routed_fixture,
    };

    #[test]
    fn owned_leak_draft_states_the_measurement_and_does_not_claim_success() {
        let snapshot = routed_fixture();
        let draft = draft_issue(
            "1.3.1",
            &snapshot,
            "why does ioctl_flags leak?\nsecond",
            false,
        );
        assert_eq!(draft.title, "[support] 1.3.1 owned leak ioctl_flags");
        assert!(draft.body.contains("Check `ioctl_flags` leaked"));
        assert!(draft.body.contains("`ioctl_flags`"));
        assert!(draft.body.contains("verdict: `partial`"));
        assert!(draft.body.contains("> why does ioctl_flags leak?"));
        assert!(draft.body.contains("> second"));
        assert!(draft.body.contains("consent: no"));
        assert!(draft.body.contains("not filed"));
        assert_no_success_claim(&draft.body);
    }

    #[test]
    fn unmeasured_gate_is_not_described_as_hiding() {
        let mut snapshot = routed_fixture();
        snapshot.gate = Some(DiagnosticGate::VpnOff);
        snapshot.native_verdict = None;
        snapshot.java_verdict = None;
        snapshot.report = None;
        let summary = snapshot.diagnostics.as_mut().unwrap();
        summary.eligibility = DiagnosticEligibility::VpnOff;
        summary.current_success = false;
        summary.evidence.as_mut().unwrap().conclusion = EvidenceConclusion::Insufficient;
        let draft = draft_issue("1.3.1", &snapshot, "is it working?", false);
        assert!(draft.title.contains("not measured (vpn_off)"));
        assert!(draft.body.contains("does not show whether hiding worked"));
        assert_no_success_claim(&draft.body);
    }

    #[test]
    fn positive_claim_stays_scoped_to_the_measurement() {
        let mut snapshot = routed_fixture();
        snapshot.native_verdict = Some(crate::snapshot::Verdict::Ok);
        snapshot.report.as_mut().unwrap().native.checks[0].outcome = CheckOutcome::HiddenBackend;
        snapshot.report.as_mut().unwrap().native.status = crate::snapshot::LayerStatus::Active {
            hidden: 4,
            leaks: 0,
        };
        {
            let summary = snapshot.diagnostics.as_mut().unwrap();
            summary.current_success = true;
            let evidence = summary.evidence.as_mut().unwrap();
            evidence.conclusion = EvidenceConclusion::NoObservedLeak;
            evidence.leaks = 0;
        }
        assert!(snapshot.allows_positive_claim());
        let draft = draft_issue("1.3.1", &snapshot, "all good?", true);
        assert!(draft.body.contains("describes this measurement only"));
        assert!(draft.body.contains("consent: yes"));
        assert!(draft.body.contains("does not store the archive"));
        assert_no_success_claim(&draft.body);
    }

    fn assert_no_success_claim(body: &str) {
        let lower = body.to_lowercase();
        assert!(!lower.contains("protection is working"));
        assert!(!lower.contains("hiding works"));
        assert!(!lower.contains("github.com/"));
    }
}
