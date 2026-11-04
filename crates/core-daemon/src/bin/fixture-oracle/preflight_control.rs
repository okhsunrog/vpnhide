//! Isolated VM entry into Core's final-byte experiment. No activation follows.
use std::path::Path;
use vpnhide_core_daemon::{
    journal::{self, preflight::PreflightPlan},
    native_boot::{BOOT_ID_PATH, NativeEnv, preflight::PreparedPreflight},
    payloads::Catalog,
};

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    let catalog = Catalog::open(Path::new("/vpnhide-preflight-catalog"))?;
    let boot = std::fs::read_to_string(BOOT_ID_PATH)?;
    let release = vpnhide_activator::sys::uname()?.release;
    let plan = PreflightPlan::from_catalog(&catalog, &release, boot.trim().into())?;
    // Qualify every candidate before the first kernel operation. No first-file
    // selection and no legacy binary with an ignored unknown module parameter.
    let prepared: Vec<_> = plan
        .candidates
        .iter()
        .map(|p| PreparedPreflight::from_catalog(&catalog, &plan, &p.sha256))
        .collect::<Result<_, _>>()?;
    let path = Path::new("/vpnhide-preflight-journal.json");
    let mut journal = match journal::read(path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => journal::Journal::default(),
        Err(e) => return Err(e.into()),
    };
    let env = NativeEnv::from_env(Some("/proc/vpnhide_ctl".into()))
        .ok_or("missing oracle environment")?;
    for candidate in &prepared {
        // Query a retained exact receipt; do not execute it again. Different
        // plans/boots and unresolved attempts still go through ledger admission.
        if journal.kmod_preflights.as_ref().is_some_and(|ledger| {
            ledger.plan == plan
                && ledger
                    .attempts
                    .iter()
                    .any(|a| a.sha256 == candidate.payload().sha256)
        }) {
            continue;
        }
        let result = candidate.dispatch_oracle(path, &mut journal, &env);
        println!(
            "PREFLIGHT_LEDGER={}",
            serde_json::to_string(&journal.kmod_preflights)?
        );
        result?;
    }
    let ledger = journal
        .kmod_preflights
        .as_ref()
        .ok_or("missing preflight receipts")?;
    println!("PREFLIGHT_LOADER_EVIDENCE={:?}", ledger.evidence());
    // Completion is loader evidence, including ambiguity. This command neither
    // selects a backend nor invokes real-load or policy-delivery entry points.
    match ledger.evidence() {
        journal::preflight::LoaderEvidence::NoSentinel
        | journal::preflight::LoaderEvidence::OneSentinel { .. }
        | journal::preflight::LoaderEvidence::MultipleSentinels { .. } => Ok(()),
        _ => Err("preflight experiment is blocked, incomplete or uncertain".into()),
    }
}
