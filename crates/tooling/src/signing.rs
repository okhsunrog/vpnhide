//! Existing Java keystore provider adapter. Rust owns package identity and the
//! envelope; this never generates a signing identity or reads private key bytes.
use clap::{Args as ClapArgs, ValueEnum};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};
use vpnhide_artifact_model::{
    archive,
    package::{Envelope, MAX_ENVELOPE_BYTES},
};
use vpnhide_bounded_process::capture_command;

#[derive(Clone, ValueEnum)]
enum Mode {
    Public,
    Sign,
}
#[derive(ClapArgs)]
pub struct Args {
    mode: Mode,
    #[arg(long)]
    keystore: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long)]
    package: Option<PathBuf>,
    #[arg(long)]
    commit: Option<String>,
    #[arg(long)]
    sequence: Option<u64>,
    #[arg(long)]
    trust: Option<PathBuf>,
    #[arg(long, default_value = "scripts/CoreDescriptorSigner.java")]
    signer: PathBuf,
}
fn provider(
    signer: &Path,
    mode: &str,
    keystore: &Path,
    source: &Path,
    destination: &Path,
) -> io::Result<()> {
    let failure =
        || io::Error::other("existing Android signing identity is unavailable or unsupported");
    let result = capture_command(
        Command::new("java")
            .arg(signer)
            .arg(mode)
            .arg(keystore)
            .arg(source)
            .arg(destination),
        None,
        std::time::Duration::from_secs(30),
        64 * 1024,
    )
    .map_err(|_| failure())?;
    // Provider output may contain aliases or sensitive paths. Never echo it.
    if result.timed_out || result.truncated || !result.output.status.success() {
        return Err(failure());
    }
    Ok(())
}
pub fn run(args: Args) -> io::Result<()> {
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let temp = tempfile::tempdir_in(parent)?;
    let source = temp.path().join("descriptor.json");
    let output = temp.path().join("provider-output");
    let public = temp.path().join("public.json");
    provider(&args.signer, "public", &args.keystore, &source, &public)?;
    let public_bytes = archive::read_regular(&public, MAX_ENVELOPE_BYTES as u64)?;
    let public_json: serde_json::Value =
        serde_json::from_slice(&public_bytes).map_err(io::Error::other)?;
    let result = match args.mode {
        Mode::Public => public_bytes,
        Mode::Sign => {
            let package = args
                .package
                .ok_or_else(|| io::Error::other("package is required"))?;
            let commit = args
                .commit
                .ok_or_else(|| io::Error::other("commit is required"))?;
            let sequence = args
                .sequence
                .ok_or_else(|| io::Error::other("sequence is required"))?;
            let trust = args
                .trust
                .ok_or_else(|| io::Error::other("compiled public trust is required"))?;
            let compiled: serde_json::Value =
                serde_json::from_slice(&archive::read_regular(&trust, MAX_ENVELOPE_BYTES as u64)?)
                    .map_err(io::Error::other)?;
            if compiled != public_json {
                return Err(io::Error::other(
                    "signer differs from Core's compiled trust identity",
                ));
            }
            let descriptor = archive::descriptor(&package, &commit, sequence)?;
            let raw = serde_json::to_string(&descriptor).map_err(io::Error::other)?;
            fs::write(&source, &raw)?;
            provider(&args.signer, "sign", &args.keystore, &source, &output)?;
            let signature = archive::read_regular(&output, 1024)?;
            if signature.is_empty() {
                return Err(io::Error::other("empty provider signature"));
            }
            let envelope = Envelope {
                descriptor: raw,
                signature: signature.iter().map(|b| format!("{b:02x}")).collect(),
            };
            serde_json::to_vec(&envelope).map_err(io::Error::other)?
        }
    };
    if result.len() > MAX_ENVELOPE_BYTES {
        return Err(io::Error::other("signing result exceeds limit"));
    }
    let mut staged = tempfile::NamedTempFile::new_in(parent)?;
    staged.write_all(&result)?;
    staged.as_file().sync_all()?;
    staged.persist(&args.output).map_err(|e| e.error)?;
    Ok(())
}
