//! Offline verifier for CI cross-language fixtures. It performs no installation.
use std::{fs, io, path::PathBuf};
use vpnhide_package_trust::{AcceptedFloor, Target, TrustStore};
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    if args.len() != 3 {
        return Err(io::Error::other("expected public trust, envelope, package").into());
    }
    let store = TrustStore::parse(&fs::read(&args[0])?)?;
    let verified = store.authenticate(&fs::read(&args[1])?)?;
    let d = verified.descriptor();
    verified.admit(
        &Target {
            source_commit: d.source_commit.clone(),
            package_sha256: d.package_sha256.clone(),
            rpc: d.rpc,
            current_journal: d.journal_max,
        },
        &AcceptedFloor::default(),
    )?;
    verified.verify_package(fs::File::open(&args[2])?)?;
    println!("authenticated exact package {}", d.package_sha256);
    Ok(())
}
