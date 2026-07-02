fn main() {
    let boot_wait = match vpnhide_activator::boot_wait_requested_from_env() {
        Ok(value) => value,
        Err(e) => {
            eprintln!("vpnhide ports activator failed: {e}");
            std::process::exit(2);
        }
    };
    if let Err(e) = vpnhide_activator::activate_ports_recorded(boot_wait) {
        eprintln!("vpnhide ports activator failed: {e}");
        std::process::exit(1);
    }
}
