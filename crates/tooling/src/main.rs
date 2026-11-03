//! Host/test tooling depends on portable formats, never the root daemon.
use clap::{Parser, Subcommand};
use std::{
    fs::{File, OpenOptions},
    io::{self, Read},
    path::{Path, PathBuf},
};
use vpnhide_artifact_model::{MAX_PAYLOAD_BYTES, elf};
mod ddk;
mod device_reset;
mod qemu;
mod signing;

#[derive(Parser)]
#[command(
    name = "vpnhide-tooling",
    version,
    about = "Shared VPN Hide artifact and test tooling"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}
#[derive(Subcommand)]
enum Command {
    DdkModpost(ddk::Args),
    DeviceReset(device_reset::Args),
    Qemu(qemu::QemuArgs),
    Distribution {
        #[command(subcommand)]
        action: DistributionAction,
    },
    Signer(signing::Args),
    Bundle {
        #[command(subcommand)]
        action: BundleAction,
    },
    Elf {
        #[command(subcommand)]
        action: ElfAction,
    },
}
#[derive(Subcommand)]
enum BundleAction {
    Kmis,
    Descriptor {
        #[arg(long)]
        package: PathBuf,
        #[arg(long)]
        commit: String,
        #[arg(long)]
        sequence: u64,
    },
    Stamp {
        #[arg(long)]
        staging: PathBuf,
        #[arg(long)]
        repo: PathBuf,
        #[arg(long)]
        component: String,
        #[arg(long)]
        version: String,
    },
    Assemble {
        #[arg(long)]
        inputs: PathBuf,
        #[arg(long)]
        output: PathBuf,
        #[arg(long)]
        commit: String,
    },
}
#[derive(Subcommand)]
enum DistributionAction {
    Select {
        #[arg(long)]
        root: PathBuf,
        #[arg(long)]
        github_output: PathBuf,
    },
    UpdateFeed {
        #[arg(long, default_value = ".")]
        repo: PathBuf,
    },
}
#[derive(Subcommand)]
enum ElfAction {
    Inspect { input: PathBuf },
    Compare { left: PathBuf, right: PathBuf },
    RequireContract { input: PathBuf },
    RequireGkiContract { input: PathBuf },
}
fn read_payload(path: &Path) -> io::Result<Vec<u8>> {
    if path.as_os_str() == "-" {
        let mut bytes = Vec::new();
        io::stdin()
            .take(MAX_PAYLOAD_BYTES + 1)
            .read_to_end(&mut bytes)?;
        return Ok(bytes);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_CLOEXEC | libc::O_NONBLOCK);
    }
    let file: File = options.open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_PAYLOAD_BYTES {
        return Err(io::Error::other("not a bounded regular payload"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PAYLOAD_BYTES + 1).read_to_end(&mut bytes)?;
    Ok(bytes)
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let result = match Cli::parse().command {
        Command::DdkModpost(args) => serde_json::to_value(ddk::run(args)?)?,
        Command::DeviceReset(args) => return device_reset::run(args).map_err(Into::into),
        Command::Qemu(args) => return qemu::run(args).map_err(Into::into),
        Command::Distribution { action } => {
            match action {
                DistributionAction::Select {
                    root,
                    github_output,
                } => vpnhide_artifact_model::distribution::github_output(&root, &github_output)?,
                DistributionAction::UpdateFeed { repo } => {
                    vpnhide_artifact_model::distribution::update_feed(&repo)?
                }
            };
            return Ok(());
        }
        Command::Signer(args) => {
            signing::run(args)?;
            return Ok(());
        }
        Command::Bundle { action } => match action {
            BundleAction::Kmis => serde_json::to_value(vpnhide_artifact_model::manifest::KMIS)?,
            BundleAction::Descriptor {
                package,
                commit,
                sequence,
            } => serde_json::to_value(vpnhide_artifact_model::archive::descriptor(
                &package, &commit, sequence,
            )?)?,
            BundleAction::Stamp {
                staging,
                repo,
                component,
                version,
            } => {
                fn git(repo: &Path, args: &[&str]) -> io::Result<String> {
                    let captured = vpnhide_bounded_process::capture_command(
                        std::process::Command::new("git")
                            .args(args)
                            .current_dir(repo),
                        None,
                        std::time::Duration::from_secs(10),
                        1024 * 1024,
                    )?;
                    if captured.timed_out || captured.truncated || !captured.output.status.success()
                    {
                        return Err(io::Error::other("git identity unavailable"));
                    }
                    String::from_utf8(captured.output.stdout).map_err(io::Error::other)
                }
                let commit = git(&repo, &["rev-parse", "HEAD"])?;
                let dirty = !git(&repo, &["status", "--porcelain", "--untracked-files=no"])?
                    .trim()
                    .is_empty();
                vpnhide_artifact_model::archive::stamp(
                    &staging,
                    &component,
                    &version,
                    commit.trim(),
                    dirty,
                )?;
                return Ok(());
            }
            BundleAction::Assemble {
                inputs,
                output,
                commit,
            } => {
                vpnhide_artifact_model::archive::assemble(&inputs, &output, &commit)?;
                return Ok(());
            }
        },
        Command::Elf { action } => match action {
            ElfAction::Inspect { input } => {
                serde_json::to_value(elf::inspect(&read_payload(&input)?)?)?
            }
            ElfAction::Compare { left, right } => serde_json::to_value(elf::compare(
                &elf::inspect(&read_payload(&left)?)?,
                &elf::inspect(&read_payload(&right)?)?,
            ))?,
            ElfAction::RequireContract { ref input }
            | ElfAction::RequireGkiContract { ref input } => {
                let bytes = read_payload(input)?;
                let result = if matches!(action, ElfAction::RequireGkiContract { .. }) {
                    elf::inspect_gki(&bytes)?
                } else {
                    elf::inspect(&bytes)?
                };
                if result.preflight_protocol != Some(1) || result.init_contract != Some(1) {
                    return Err(io::Error::other(
                        "payload lacks the verified current preflight/closed-init capability",
                    )
                    .into());
                }
                serde_json::to_value(result)?
            }
        },
    };
    println!("{}", serde_json::to_string(&result)?);
    Ok(())
}
