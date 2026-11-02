//! Offline inspection only. This executable has no install/flash/device-write command.
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
fn run() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args_os().skip(1).collect();
    let [command, image] = args.as_slice() else {
        return Err("usage: vpnhide_provisioning inspect-kernel <extracted-arm64-Image>".into());
    };
    if command != "inspect-kernel" {
        return Err("only offline inspect-kernel is supported".into());
    }
    let input = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK | libc::O_CLOEXEC)
        .open(image)?;
    if !input.metadata()?.is_file() || input.metadata()?.len() > 256 * 1024 * 1024 {
        return Err("expected a bounded regular kernel Image".into());
    }
    let mut bytes = Vec::new();
    input.take(256 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    println!(
        "{}",
        serde_json::to_string(&vpnhide_provisioning::kernel::inspect_kernel(&bytes)?)?
    );
    Ok(())
}
fn main() {
    if let Err(error) = run() {
        eprintln!("provisioning: {error}");
        std::process::exit(1);
    }
}
