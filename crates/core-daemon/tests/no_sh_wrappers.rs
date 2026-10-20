//! Core never spawns a shell, and that includes AOSP's sh wrapper scripts: `pm` is
//! `#!/system/bin/sh` + `cmd package "$@"`, and `am`, `settings`, `input`, `svc`, `wm`,
//! `content`, `appops`, `monkey` are the same shape. This scans the crates core is built
//! from for a spawn of any of them (or of `sh` itself).

use std::path::Path;

const WRAPPERS: &[&str] = &[
    "sh", "pm", "am", "settings", "input", "svc", "wm", "content", "appops", "monkey", "ime",
];

fn scan(dir: &Path, hits: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            scan(&path, hits);
            continue;
        }
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        for (n, line) in text.lines().enumerate() {
            for name in WRAPPERS {
                for spawn in [
                    format!("Command::new(\"{name}\")"),
                    format!("run(\"{name}\""),
                    format!("Command::new(\"/system/bin/{name}\")"),
                ] {
                    if line.contains(&spawn) {
                        hits.push(format!("{}:{}: {}", path.display(), n + 1, line.trim()));
                    }
                }
            }
        }
    }
}

#[test]
fn core_spawns_no_sh_wrapper() {
    let crates = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let mut hits = Vec::new();
    for krate in ["core-daemon/src", "activator/src", "core-rpc/src"] {
        scan(&crates.join(krate), &mut hits);
    }
    assert!(
        hits.is_empty(),
        "core spawns an sh wrapper:\n{}",
        hits.join("\n")
    );
}
