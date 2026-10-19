//! `vpnhide-core probe …` / `vpnhide-core observe …`: one-shot root probes.
//!
//! Core runs these as a child of itself (see `diagnostics`), so a probe that hangs or panics
//! is bounded and killed without touching the daemon. The JSON they print is the contract
//! shared with the app's parsers (`fixtures/core-probes`).

mod observations;
mod probe;

/// Dispatch a probe subcommand. `None` when `args` is not one (the caller runs the daemon).
pub fn run(args: &[String]) -> Option<i32> {
    let code = match args {
        [command, mode] if command == "probe" && mode == "checks" => probe::checks(&[]),
        [command, mode, rest @ ..] if command == "probe" && mode == "routing" => {
            probe::routing(rest)
        }
        [command, mode, rest @ ..] if command == "observe" && mode == "app-vpn-state" => {
            probe::app_vpn_state(rest)
        }
        [command, mode] if command == "observe" && mode == "kpm-list" => {
            observations::kpm_list(&[])
        }
        [command, mode] if command == "observe" && mode == "kpm-state" => observations::kpm_state(),
        [command, ..] if command == "probe" || command == "observe" => {
            eprintln!(
                "usage: vpnhide-core probe checks | probe routing --uid <uid> [--vpn-ifaces <list>] | observe app-vpn-state --uid <uid> | observe kpm-list | observe kpm-state"
            );
            2
        }
        _ => return None,
    };
    Some(code)
}
