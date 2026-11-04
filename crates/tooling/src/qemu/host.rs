//! Host orchestration is direct argv execution. Guest policy/results are typed.
use super::{
    model::{self, Backend, Mode, Phase, Plan, Probe, Probes},
    verify,
};
use clap::{Args as ClapArgs, ValueEnum};
use std::{
    env,
    ffi::OsString,
    fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};
use vpnhide_artifact_model::{
    archive::{FileIdentity, read_regular},
    initramfs,
};
use vpnhide_bounded_process::{CapturedCommand, capture_command};
#[derive(Clone, Copy, ValueEnum)]
pub enum BackendArg {
    Kmod,
    Builtin,
    Kpm,
    Userspace,
}
#[derive(ClapArgs)]
pub struct Args {
    #[arg(long, value_enum, default_value = "kmod")]
    pub backend: BackendArg,
    #[arg(default_value = "android12-5.10")]
    pub kmi: String,
    #[arg(long)]
    pub repo: Option<PathBuf>,
    /// Fresh evidence directory. Existing directories are refused.
    #[arg(long)]
    pub work_dir: Option<PathBuf>,
}
fn error(s: impl ToString) -> io::Error {
    io::Error::other(s.to_string())
}
fn env_path(name: &str, default: impl Into<PathBuf>) -> PathBuf {
    env::var_os(name)
        .map(PathBuf::from)
        .unwrap_or_else(|| default.into())
}
fn nonempty(name: &str) -> bool {
    env::var_os(name).is_some_and(|v| !v.is_empty())
}
fn success(c: &CapturedCommand) -> bool {
    !c.timed_out && !c.truncated && c.output.status.success()
}
fn execute(
    cmd: &mut Command,
    timeout: u64,
    limit: usize,
    log: &Path,
) -> io::Result<CapturedCommand> {
    let r = capture_command(cmd, None, Duration::from_secs(timeout), limit)?;
    let mut bytes = r.output.stdout.clone();
    bytes.extend_from_slice(b"\nHOST_STDERR\n");
    bytes.extend_from_slice(&r.output.stderr);
    fs::write(log, bytes)?;
    Ok(r)
}
fn copy_file(root: &Path, name: &str, source: &Path, executable: bool) -> io::Result<()> {
    initramfs::place(
        root,
        name,
        &read_regular(source, 128 * 1024 * 1024)?,
        executable,
    )
}
fn download(url: &str, path: &Path) -> io::Result<()> {
    if path.is_file() {
        return Ok(());
    }
    let parent = path
        .parent()
        .ok_or_else(|| error("download path has no parent"))?;
    fs::create_dir_all(parent)?;
    let tmp = tempfile::NamedTempFile::new_in(parent)?;
    let r = capture_command(
        Command::new("curl")
            .args([
                "--fail",
                "--location",
                "--silent",
                "--show-error",
                "--max-time",
                "90",
                "--output",
            ])
            .arg(tmp.path())
            .arg(url),
        None,
        Duration::from_secs(95),
        64 * 1024,
    )?;
    if !success(&r) {
        return Err(error(format!("input download failed: {url}")));
    }
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
struct ProbeInput {
    probe: Probe,
    path: Option<PathBuf>,
}
fn probe(
    repo: &Path,
    work: &Path,
    prefix: &str,
    name: &str,
    source: &str,
    compat: bool,
) -> io::Result<ProbeInput> {
    let required = nonempty(&format!("VPNHIDE_{prefix}_REQUIRED"));
    let binary = env::var_os(format!("VPNHIDE_{prefix}_BIN"))
        .filter(|v| !v.is_empty())
        .map(PathBuf::from);
    let path = if let Some(p) = binary {
        if !p.is_file() {
            return Err(error(format!("explicit {prefix} probe is missing")));
        }
        Some(p)
    } else {
        let compiler = env::var_os(format!("VPNHIDE_{prefix}_CC"))
            .or_else(|| {
                if !compat {
                    env::var_os("VPNHIDE_GAI_CC")
                } else {
                    None
                }
            })
            .or_else(|| {
                env::var_os("ANDROID_NDK_HOME").map(|p| {
                    PathBuf::from(p)
                        .join("toolchains/llvm/prebuilt/linux-x86_64/bin")
                        .join(if compat {
                            "armv7a-linux-androideabi24-clang"
                        } else {
                            "aarch64-linux-android24-clang"
                        })
                        .into_os_string()
                })
            });
        if let Some(cc) = compiler {
            let p = work.join(name);
            let mut cmd = Command::new(cc);
            cmd.args(["-static", "-O2", "-Wall", "-Wextra", "-Werror", "-o"])
                .arg(&p)
                .arg(repo.join("kmod/test").join(source));
            let r = execute(
                &mut cmd,
                90,
                128 * 1024,
                &work.join(format!("build-{name}.log")),
            )?;
            if !success(&r) {
                return Err(error(format!(
                    "{prefix} probe compilation failed; see retained log"
                )));
            }
            Some(p)
        } else {
            None
        }
    };
    if required && path.is_none() {
        return Err(error(format!("required {prefix} probe unavailable")));
    }
    if let Some(p) = &path {
        let bytes = read_regular(p, 32 * 1024 * 1024)?;
        let class = if compat { 1 } else { 2 };
        let machine = if compat { 40u16 } else { 183u16 };
        if bytes.get(..5) != Some(&[0x7f, b'E', b'L', b'F', class])
            || bytes.get(5) != Some(&1)
            || bytes.get(18..20) != Some(machine.to_le_bytes().as_slice())
        {
            return Err(error(format!("wrong ABI for {prefix} probe")));
        }
    }
    Ok(ProbeInput {
        probe: Probe {
            present: path.is_some(),
            required,
        },
        path,
    })
}
fn phase_report(plan: &Plan, serial: &[u8]) -> io::Result<model::PhaseReport> {
    let report: model::PhaseReport =
        super::decode_serial(&plan.session, "VPNHIDE_KPM_PHASE ", serial)?;
    report.validate(plan)?;
    Ok(report)
}
fn boot(plan: &Plan, image: &Path, root: &Path, work: &Path, tag: &str) -> io::Result<Vec<u8>> {
    initramfs::place(
        root,
        "vpnhide-plan.json",
        &serde_json::to_vec(plan).map_err(error)?,
        false,
    )?;
    let initrd = work.join(format!("initramfs-{tag}.gz"));
    initramfs::pack(root, &initrd)?;
    fs::write(
        work.join(format!("plan-{tag}.json")),
        serde_json::to_vec_pretty(plan).map_err(error)?,
    )?;
    let mut cmd = Command::new(
        env::var_os("VPNHIDE_QEMU_BIN").unwrap_or_else(|| OsString::from("qemu-system-aarch64")),
    );
    cmd.args(["-machine", "virt", "-cpu"])
        .arg(env::var_os("VPNHIDE_QEMU_CPU").unwrap_or_else(|| OsString::from("max")))
        .args([
            "-accel",
            "tcg,thread=multi,tb-size=1024",
            "-smp",
            "4",
            "-m",
            "2G",
            "-kernel",
        ])
        .arg(image)
        .arg("-initrd")
        .arg(&initrd)
        .arg("-append")
        .arg(format!(
            "console=ttyAMA0 rodata=off panic=-1 rdinit=/init vpnhide_harness={}{}",
            plan.session,
            if matches!(
                plan.mode,
                Mode::FixtureRaw | Mode::FixtureProtected | Mode::CorePreflight
            ) {
                " vpnhide_fixture_oracle=1"
            } else {
                ""
            }
        ))
        .args([
            "-netdev",
            "user,id=n0",
            "-device",
            "virtio-net-pci,netdev=n0,romfile=",
            "-device",
            "virtio-rng-pci",
            "-display",
            "none",
            "-monitor",
            "none",
            "-no-reboot",
            "-serial",
            "stdio",
        ]);
    let r = execute(
        &mut cmd,
        300,
        32 * 1024 * 1024,
        &work.join(format!("serial-{tag}.log")),
    )?;
    if !success(&r) {
        return Err(error(format!(
            "QEMU failed/timed out/truncated in {tag}; retained evidence {}",
            work.display()
        )));
    }
    for line in String::from_utf8_lossy(&r.output.stdout).lines() {
        if [
            "RESULT ",
            "SUMMARY ",
            "HARNESS_ERROR",
            "IPROUTE2",
            "INSMOD",
            "VPNHIDE_HARNESS_END",
        ]
        .iter()
        .any(|p| line.starts_with(p))
        {
            println!("{line}");
        }
    }
    Ok(r.output.stdout)
}
pub fn run(args: Args) -> io::Result<()> {
    let repo = args.repo.unwrap_or(env::current_dir()?).canonicalize()?;
    if !repo.join("kmod/test/init.sh").is_file() {
        return Err(error("run from repository root or supply --repo"));
    }
    if args.kmi.len() > 128
        || !args
            .kmi
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._".contains(&b))
        || args.kmi.starts_with('.')
    {
        return Err(error("invalid kernel cache identity"));
    }
    let backend = match args.backend {
        BackendArg::Kmod => Backend::Kmod,
        BackendArg::Builtin => Backend::Builtin,
        BackendArg::Kpm => Backend::Kpm,
        BackendArg::Userspace => Backend::Userspace,
    };
    let cache = repo.join(if backend == Backend::Builtin {
        "builtin/test/.cache"
    } else {
        "kmod/test/.cache"
    });
    fs::create_dir_all(&cache)?;
    let work = if let Some(path) = args.work_dir {
        fs::create_dir(&path)?;
        path.canonicalize()?
    } else {
        tempfile::Builder::new()
            .prefix("vpnhide-qemu-")
            .tempdir()?
            .keep()
    };
    println!("QEMU evidence {}", work.display());
    let result = (|| -> io::Result<()> {
        let source_image = env_path("VPNHIDE_QEMU_IMAGE", cache.join(&args.kmi).join("Image"));
        let image = work.join("Image");
        fs::write(&image, read_regular(&source_image, 256 * 1024 * 1024)?)?;
        let rootfs = env_path(
            "VPNHIDE_QEMU_ROOTFS",
            cache.join("alpine-minirootfs-3.21.2-aarch64.tar.gz"),
        );
        download(
            "https://dl-cdn.alpinelinux.org/alpine/v3.21/releases/aarch64/alpine-minirootfs-3.21.2-aarch64.tar.gz",
            &rootfs,
        )?;
        let guest = env_path(
            "VPNHIDE_QEMU_GUEST_BIN",
            repo.join("target/aarch64-unknown-linux-musl/release/vpnhide-tooling"),
        );
        let guest_bytes = read_regular(&guest, 64 * 1024 * 1024)?;
        if guest_bytes.get(..5) != Some(&[0x7f, b'E', b'L', b'F', 2])
            || guest_bytes.get(18..20) != Some(&[183, 0])
        {
            return Err(error("build the static AArch64 tooling guest first"));
        }
        let gai = probe(&repo, &work, "GAI", "gai", "gai-probe.c", false)?;
        let ifconf = probe(&repo, &work, "IFC", "ifconf", "ifconf-probe.c", false)?;
        let bind = probe(&repo, &work, "BIND", "bind-probe", "bind-probe.c", false)?;
        let ioctl = probe(
            &repo,
            &work,
            "IOC",
            "iface-ioctl",
            "iface-ioctl-probe.c",
            false,
        )?;
        let compat = if backend != Backend::Kpm {
            probe(
                &repo,
                &work,
                "BIND32",
                "bind-probe32",
                "bind-probe-compat.c",
                true,
            )?
        } else {
            ProbeInput {
                probe: Probe::default(),
                path: None,
            }
        };
        let preflight = if backend == Backend::Kmod {
            probe(
                &repo,
                &work,
                "PREFLIGHT",
                "preflight-probe",
                "preflight-probe.c",
                false,
            )?
        } else {
            ProbeInput {
                probe: Probe::default(),
                path: None,
            }
        };
        let mode = if nonempty("VPNHIDE_QEMU_NATIVE_UNIT_BIN") {
            Mode::NativeUnit
        } else if nonempty("VPNHIDE_QEMU_CORE_PREFLIGHT") {
            Mode::CorePreflight
        } else if nonempty("VPNHIDE_QEMU_PREFLIGHT_ONLY") {
            Mode::Preflight
        } else {
            match env::var("VPNHIDE_QEMU_NATIVE_FIXTURE_MODE").as_deref() {
                Ok("raw") => Mode::FixtureRaw,
                Ok("protected") => Mode::FixtureProtected,
                Ok("") | Err(_) => Mode::Full,
                Ok(_) => return Err(error("fixture mode must be raw or protected")),
            }
        };
        let root = work.join("rootfs");
        initramfs::extract(&rootfs, &root)?;
        if let Some(ip) = env::var_os("VPNHIDE_IPROUTE_BIN") {
            copy_file(&root, "vpnhide-ip", &PathBuf::from(ip), true)?;
        }
        let payload = if backend == Backend::Builtin {
            None
        } else if backend == Backend::Userspace {
            let path = env_path("VPNHIDE_QEMU_NATIVE_UNIT_BIN", PathBuf::new());
            let bytes = read_regular(&path, 128 * 1024 * 1024)?;
            initramfs::place(&root, "native-unit-tests", &bytes, true)?;
            Some(FileIdentity::of(&bytes).sha256)
        } else {
            let source = if backend == Backend::Kpm {
                env_path("VPNHIDE_KPM", repo.join("kmod/vpnhide.kpm"))
            } else {
                env_path(
                    "VPNHIDE_QEMU_KO",
                    cache.join(&args.kmi).join("vpnhide_kmod.ko"),
                )
            };
            if backend == Backend::Kpm && !source.is_file() {
                let r = execute(
                    Command::new("make")
                        .arg("-C")
                        .arg(repo.join("kmod"))
                        .arg("kpm"),
                    120,
                    1024 * 1024,
                    &work.join("build-kpm.log"),
                )?;
                if !success(&r) {
                    return Err(error("KPM build failed; see retained evidence"));
                }
            }
            let bytes = read_regular(&source, 128 * 1024 * 1024)?;
            initramfs::place(
                &root,
                if backend == Backend::Kpm {
                    "vpnhide.kpm"
                } else {
                    "vpnhide_kmod.ko"
                },
                &bytes,
                false,
            )?;
            Some(FileIdentity::of(&bytes).sha256)
        };
        let mut nonce = [0u8; 32];
        use std::io::Read;
        fs::File::open("/dev/urandom")?.read_exact(&mut nonce)?;
        let session = nonce.iter().map(|b| format!("{b:02x}")).collect();
        let mut plan = Plan {
            schema: model::REPORT_SCHEMA,
            session,
            backend,
            mode,
            payload_sha256: payload,
            probes: Probes {
                gai: gai.probe,
                ifconf: ifconf.probe,
                bind: bind.probe,
                ioctl: ioctl.probe,
                compat: compat.probe,
                preflight: preflight.probe,
            },
            kpm_phase: None,
        };
        plan.validate()?;
        initramfs::place(
            &root,
            "init",
            &read_regular(&repo.join("kmod/test/init.sh"), 16 * 1024)?,
            true,
        )?;
        initramfs::place(&root, "vpnhide-tooling", &guest_bytes, true)?;
        for (name, input) in [
            ("gai", gai),
            ("ifconf", ifconf),
            ("bind-probe", bind),
            ("iface-ioctl", ioctl),
            ("bind-probe32", compat),
            ("preflight-probe", preflight),
        ] {
            if let Some(p) = input.path {
                copy_file(&root, name, &p, true)?;
            }
        }
        if matches!(
            mode,
            Mode::FixtureRaw | Mode::FixtureProtected | Mode::CorePreflight
        ) {
            let oracle = env_path("VPNHIDE_NATIVE_FIXTURE_BIN", PathBuf::new());
            copy_file(&root, "native-fixture-oracle", &oracle, true)?;
        }
        let mut staged = std::collections::BTreeMap::new();
        if mode == Mode::CorePreflight {
            let source = env_path("VPNHIDE_QEMU_PREFLIGHT_CATALOG", PathBuf::new());
            let bytes = read_regular(
                &source.join("payloads/manifest.json"),
                vpnhide_artifact_model::manifest::MAX_MANIFEST_BYTES,
            )?;
            let manifest = vpnhide_artifact_model::manifest::Manifest::parse(&bytes)?;
            let manifest_path = "vpnhide-preflight-catalog/payloads/manifest.json";
            initramfs::place(&root, manifest_path, &bytes, false)?;
            staged.insert(manifest_path.to_owned(), FileIdentity::of(&bytes));
            let mut total = 0u64;
            // Host staging consumes the shared manifest grammar. The Core in
            // the guest independently verifies actual final bytes/capabilities.
            for payload in &manifest.payloads {
                let relative = payload.implementation.relative_path()?;
                let bytes = read_regular(
                    &source.join(&relative),
                    vpnhide_artifact_model::MAX_PAYLOAD_BYTES,
                )?;
                total += bytes.len() as u64;
                if total > vpnhide_artifact_model::manifest::MAX_BUNDLE_BYTES {
                    return Err(error("preflight catalog exceeds staging budget"));
                }
                let destination = format!("vpnhide-preflight-catalog/{relative}");
                initramfs::place(&root, &destination, &bytes, false)?;
                staged.insert(destination, FileIdentity::of(&bytes));
            }
        }
        for name in [
            "init",
            "vpnhide-tooling",
            "vpnhide-ip",
            "vpnhide_kmod.ko",
            "vpnhide.kpm",
            "native-fixture-oracle",
            "native-unit-tests",
            "gai",
            "ifconf",
            "bind-probe",
            "bind-probe32",
            "iface-ioctl",
            "preflight-probe",
        ] {
            if root.join(name).exists() {
                staged.insert(
                    name.to_owned(),
                    FileIdentity::of(&read_regular(&root.join(name), 128 * 1024 * 1024)?),
                );
            }
        }
        fs::write(work.join("input-identities.json"),serde_json::to_vec_pretty(&serde_json::json!({"kernel":FileIdentity::of(&read_regular(&image,256*1024*1024)?),"guest":FileIdentity::of(&guest_bytes),"payload":plan.payload_sha256,"staged":staged})).map_err(error)?)?;
        if backend == Backend::Kpm {
            let kp = env_path("VPNHIDE_KP_BIN", cache.join("kp"));
            let mut phases = Vec::new();
            for phase in [Phase::Raw, Phase::Protected] {
                plan.kpm_phase = Some(phase);
                let tag = if phase == Phase::Raw {
                    "raw"
                } else {
                    "protected"
                };
                let patched = work.join(format!("Image-{tag}"));
                use vpnhide_protocol::{
                    Target,
                    hook_ids::{KERNEL_HOOK_MASK, KMOD_HOOK_MASK},
                };
                let targets = if phase == Phase::Protected {
                    vec![Target {
                        uid: model::TARGET_UID,
                        hookmask: KERNEL_HOOK_MASK | KMOD_HOOK_MASK,
                    }]
                } else {
                    Vec::new()
                };
                let wire = vpnhide_protocol::format_config(false, 0, &targets);
                let mut cmd = Command::new(kp.join("kptools-linux"));
                cmd.args(["-p", "-i"])
                    .arg(&image)
                    .arg("-k")
                    .arg(kp.join("kpimg-linux"))
                    .args(["-S", "vpnhide-qemu-test", "-M"])
                    .arg(root.join("vpnhide.kpm"))
                    .args(["-T", "kpm", "-A"])
                    .arg(wire)
                    .arg("-o")
                    .arg(&patched);
                let r = execute(
                    &mut cmd,
                    120,
                    1024 * 1024,
                    &work.join(format!("patch-{tag}.log")),
                )?;
                if !success(&r) {
                    return Err(error("KernelPatch image preparation failed"));
                }
                let serial = boot(&plan, &patched, &root, &work, tag)?;
                phases.push(phase_report(&plan, &serial)?);
            }
            plan.kpm_phase = None;
            let report = model::compare_kpm(&plan, &phases[0], &phases[1])?;
            fs::write(
                work.join("report.json"),
                serde_json::to_vec_pretty(&report).map_err(error)?,
            )?;
            println!("KPM verified vectors={}", report.rows.len());
        } else {
            let serial = boot(&plan, &image, &root, &work, "run")?;
            verify(&plan, &serial)?;
        }
        Ok(())
    })();
    // Evidence survives every failure. Large staging files can be discarded
    // after a verified run unless the caller explicitly requested retention.
    if result.is_ok() && !nonempty("VPNHIDE_QEMU_KEEP_WORK") {
        let _ = fs::remove_dir_all(work.join("rootfs"));
        for name in [
            "Image",
            "Image-raw",
            "Image-protected",
            "initramfs-run.gz",
            "initramfs-raw.gz",
            "initramfs-protected.gz",
        ] {
            let _ = fs::remove_file(work.join(name));
        }
    }
    result
}
