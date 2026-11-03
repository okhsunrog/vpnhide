//! Host compiler adapter for normal module symbol/version checking. Some DDK
//! images disable two modpost statements; never modify the installed DDK tree
//! or compensate by inventing ELF CRC records or relaxing the kernel loader.
use clap::Args as ClapArgs;
use std::{fs, io, path::PathBuf, process::Command, time::Duration};
use vpnhide_artifact_model::archive::{FileIdentity, read_regular};

#[derive(ClapArgs)]
pub struct Args {
    #[arg(long)]
    kdir: PathBuf,
    #[arg(long)]
    output: PathBuf,
    #[arg(long, default_value = "cc")]
    compiler: PathBuf,
}

#[derive(serde::Serialize)]
pub struct Report {
    output: PathBuf,
    original_source: FileIdentity,
    qualified_source: FileIdentity,
    restored_checks: usize,
}

fn invalid(message: impl ToString) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

/// A narrowly recognized source repair, not a general C rewrite. Recompile the
/// normal checker even when both statements already exist: a prebuilt binary
/// need not correspond to the accompanying source. Final ELF validation is a
/// separate requirement and never trusts this source report.
fn qualify(source: &str) -> io::Result<(String, usize)> {
    let mut seen = [0; 2];
    let mut restored = 0;
    let mut result = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        let trimmed = line.trim();
        let (commented, statement) = match trimmed.strip_prefix("//") {
            Some(body) => (true, body.trim()),
            None => (false, trimmed),
        };
        let check = match statement {
            "check_exports(mod);" | "err |= check_exports(mod);" => Some(0),
            "s->module = exp->module;" => Some(1),
            _ => None,
        };
        if let Some(check) = check {
            seen[check] += 1;
            if commented {
                result.push_str(&line.replacen("//", "", 1));
                restored += 1;
                continue;
            }
        }
        result.push_str(line);
    }
    if seen != [1, 1] {
        return Err(invalid(
            "unsupported or ambiguous modpost export/version checker",
        ));
    }
    Ok((result, restored))
}

pub fn run(args: Args) -> io::Result<Report> {
    let kernel = args.kdir.canonicalize()?;
    let source = if kernel.join("scripts/mod/modpost.c").is_file() {
        kernel.clone()
    } else {
        kernel.join("source").canonicalize()?
    };
    let original = read_regular(&source.join("scripts/mod/modpost.c"), 256 * 1024)?;
    let text = std::str::from_utf8(&original).map_err(invalid)?;
    let (qualified, restored_checks) = qualify(text)?;
    let parent = args
        .output
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(std::path::Path::new("."));
    fs::create_dir_all(parent)?;
    let staging = tempfile::tempdir_in(parent)?;
    let prepared = staging.path().join("modpost.c");
    let binary = staging.path().join("modpost");
    fs::write(&prepared, qualified.as_bytes())?;
    let mut command = Command::new(&args.compiler);
    command.args([
        "-O2",
        "-std=gnu11",
        "-Wall",
        "-Wextra",
        "-Wmissing-prototypes",
        "-Wstrict-prototypes",
    ]);
    for include in [
        source.join("scripts/mod"),
        source.join("scripts/include"),
        kernel.join("scripts/mod"),
    ] {
        command.arg("-I").arg(include);
    }
    command.arg(&prepared);
    for unit in ["file2alias.c", "sumversion.c", "symsearch.c"] {
        let path = source.join("scripts/mod").join(unit);
        if unit == "symsearch.c" && !path.exists() {
            continue; // Older supported kernels have no symsearch translation unit.
        }
        read_regular(&path, 512 * 1024)?;
        command.arg(path);
    }
    command.arg("-o").arg(&binary);
    let captured = vpnhide_bounded_process::capture_command(
        &mut command,
        None,
        Duration::from_secs(120),
        64 * 1024,
    )?;
    if captured.timed_out || captured.truncated || !captured.output.status.success() {
        return Err(invalid(format!(
            "qualified modpost compiler failed: {}",
            String::from_utf8_lossy(&captured.output.stderr)
        )));
    }
    if !binary.metadata()?.is_file() {
        return Err(invalid("qualified modpost compiler produced no executable"));
    }
    fs::rename(binary, &args.output)?;
    Ok(Report {
        output: args.output,
        original_source: FileIdentity::of(&original),
        qualified_source: FileIdentity::of(qualified.as_bytes()),
        restored_checks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restores_only_the_known_disabled_checks_and_preserves_other_source_bytes() {
        let old = "/* unchanged */\n//\t\tcheck_exports(mod);\n//\t\ts->module = exp->module;\n";
        assert_eq!(
            qualify(old).unwrap(),
            (
                "/* unchanged */\n\t\tcheck_exports(mod);\n\t\ts->module = exp->module;\n".into(),
                2,
            )
        );
        let normal = "\tcheck_exports(mod);\n\t\ts->module = exp->module;\n";
        assert_eq!(qualify(normal).unwrap(), (normal.into(), 0));
        let older = "// err |= check_exports(mod);\ns->module = exp->module;\n";
        assert_eq!(qualify(older).unwrap().1, 1);
    }

    #[test]
    fn missing_duplicate_and_similar_but_different_statements_are_not_a_supported_checker() {
        for bad in [
            "// check_exports(mod);\n",
            "check_exports(mod);\ncheck_exports(mod);\ns->module = exp->module;\n",
            "check_exports(mod);\n// s->module = other;\n",
            "check_exports(other);\ns->module = exp->module;\n",
            "/* check_exports(mod); */\ns->module = exp->module;\n",
        ] {
            assert!(qualify(bad).is_err());
        }
    }
}
