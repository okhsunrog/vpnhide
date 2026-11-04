//! Explicit isolated-kernel test executor; never an Android production daemon.
mod guest;
mod host;
mod model;
use clap::{Args, Subcommand};
use std::{io, path::PathBuf};
#[derive(Args)]
pub struct QemuArgs {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    Run(host::Args),
    Guest,
    Observe {
        kind: guest::Observation,
    },
    Verify {
        #[arg(long)]
        plan: PathBuf,
        #[arg(long)]
        log: PathBuf,
    },
}
pub fn run(args: QemuArgs) -> io::Result<()> {
    match args.command {
        Action::Run(args) => host::run(args),
        Action::Guest => guest::run(),
        Action::Observe { kind } => guest::observe(kind),
        Action::Verify { plan, log } => {
            let p: model::Plan = serde_json::from_slice(
                &vpnhide_artifact_model::archive::read_regular(&plan, 64 * 1024)?,
            )
            .map_err(io::Error::other)?;
            let bytes = vpnhide_artifact_model::archive::read_regular(&log, 32 * 1024 * 1024)?;
            verify(&p, &bytes)
        }
    }
}
fn decode_serial<T: serde::de::DeserializeOwned>(
    session: &str,
    prefix: &str,
    serial: &[u8],
) -> io::Result<T> {
    if serial.len() > 32 * 1024 * 1024 {
        return Err(io::Error::other("serial exceeds budget"));
    }
    let text = std::str::from_utf8(serial).map_err(io::Error::other)?;
    if !model::panic_lines(text).is_empty() {
        return Err(io::Error::other("kernel panic/oops in serial log"));
    }
    let mut report = None;
    let mut ended = false;
    for line in text.lines() {
        let line = line.trim_end_matches('\r');
        if let Some(raw) = line.strip_prefix(prefix) {
            if ended || report.is_some() || raw.len() > 64 * 1024 {
                return Err(io::Error::other("duplicate, late or excessive report"));
            }
            report = Some(serde_json::from_str(raw).map_err(io::Error::other)?);
        }
        if let Some(actual) = line.strip_prefix("VPNHIDE_HARNESS_END ") {
            if actual != session || ended || report.is_none() {
                return Err(io::Error::other(
                    "invalid, duplicate or premature end marker",
                ));
            }
            ended = true;
        }
    }
    if !ended {
        return Err(io::Error::other("guest did not complete"));
    }
    report.ok_or_else(|| io::Error::other("missing typed report"))
}
pub fn verify(plan: &model::Plan, serial: &[u8]) -> io::Result<()> {
    let report: model::Report = decode_serial(&plan.session, "VPNHIDE_HARNESS_REPORT ", serial)?;
    report.validate(plan)?;
    let skipped = report
        .rows
        .iter()
        .filter(|r| r.verdict == model::Verdict::Skip)
        .count();
    println!(
        "QEMU verified session={} vectors={} skipped={skipped} scope={:?}",
        plan.session,
        report.rows.len(),
        plan.mode
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn core_preflight_requires_both_oracles_and_forbids_real_activation() {
        let plan = model::Plan {
            schema: model::REPORT_SCHEMA,
            session: "a".repeat(64),
            backend: model::Backend::Kmod,
            mode: model::Mode::CorePreflight,
            payload_sha256: Some("b".repeat(64)),
            probes: model::Probes {
                preflight: model::Probe {
                    present: true,
                    required: true,
                },
                ..Default::default()
            },
            kpm_phase: None,
        };
        let mut report = model::Report::new(&plan);
        report.setup_ok = true;
        report.rows.push(model::ResultRow::new(
            "loader_preflight",
            model::Verdict::Pass,
            "independent C syscall oracle",
        ));
        assert!(report.validate(&plan).is_err());
        report.rows.push(model::ResultRow::new(
            "core_preflight_dispatch",
            model::Verdict::Pass,
            "Core durable experiment",
        ));
        report.validate(&plan).unwrap();
        report.loaded = true;
        assert!(report.validate(&plan).is_err());
        report.loaded = false;
        report.registered = 1;
        assert!(report.validate(&plan).is_err());
        let mut wrong_backend = plan.clone();
        wrong_backend.backend = model::Backend::Kpm;
        assert!(wrong_backend.validate().is_err());
        let mut missing_oracle = plan;
        missing_oracle.probes.preflight = Default::default();
        assert!(missing_oracle.validate().is_err());
    }

    #[test]
    fn summary_only_and_panic_after_report_cannot_pass() {
        let p = model::Plan {
            schema: model::REPORT_SCHEMA,
            session: "a".repeat(64),
            backend: model::Backend::Kmod,
            mode: model::Mode::Preflight,
            payload_sha256: Some("b".repeat(64)),
            probes: model::Probes {
                preflight: model::Probe {
                    present: true,
                    required: true,
                },
                ..Default::default()
            },
            kpm_phase: None,
        };
        assert!(verify(&p, b"SUMMARY pass=1 fail=0 panic=0 registered=0\n").is_err());
        let mut r = model::Report::new(&p);
        r.setup_ok = true;
        r.rows.push(model::ResultRow::new(
            "loader_preflight",
            model::Verdict::Pass,
            "exact sentinel/postconditions",
        ));
        let text = format!(
            "VPNHIDE_HARNESS_REPORT {}\nVPNHIDE_HARNESS_END {}\n",
            serde_json::to_string(&r).unwrap(),
            p.session
        );
        verify(&p, text.as_bytes()).unwrap();
        let mut exposed = r.clone();
        exposed.loaded = true;
        assert!(exposed.validate(&p).is_err());
        assert!(verify(&p, format!("{text}Kernel panic - not syncing\n").as_bytes()).is_err());
        assert!(verify(&p, format!("{text}{text}").as_bytes()).is_err());
        assert!(
            verify(
                &p,
                format!("{text}VPNHIDE_HARNESS_END {}\n", p.session).as_bytes()
            )
            .is_err()
        );
        assert!(
            verify(
                &p,
                format!("VPNHIDE_HARNESS_END {}\n{text}", p.session).as_bytes()
            )
            .is_err()
        );
    }
}
