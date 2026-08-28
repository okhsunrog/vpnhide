use std::{env, process};

use vpnhide_activator::{Result, activate_kpatch, boot_service_kpatch, uninstall_kpatch};

fn main() {
    if let Err(e) = run() {
        eprintln!("vpnhide kpatch activator failed: {e}");
        process::exit(1);
    }
}

fn run() -> Result<()> {
    // The in-tree backend has no module to load (it is built into the kernel),
    // so there is no `boot-load` — only config delivery + liveness. The app reads
    // status/stats straight from /proc/vpnhide_ctl, same as the .ko.
    match env::args().skip(1).collect::<Vec<_>>().as_slice() {
        [] => activate_kpatch(),
        [command] if command == "boot-service" => boot_service_kpatch(),
        [command] if command == "uninstall" => uninstall_kpatch(),
        _ => Err("usage: activator [boot-service|uninstall]".into()),
    }
}
