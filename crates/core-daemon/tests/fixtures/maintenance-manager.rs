//! A real ELF adversary for the manager process boundary. Never shipped.
use std::{
    env, fs,
    path::{Path, PathBuf},
    process,
};
fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).unwrap();
    for e in fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let p = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_tree(&e.path(), &p)
        } else {
            fs::copy(e.path(), p).unwrap();
        }
    }
}
fn main() {
    let exe = fs::read_link("/proc/self/exe").unwrap();
    let adb = exe.parent().unwrap();
    let args: Vec<_> = env::args().skip(1).collect();
    let kind = if exe.file_name().unwrap() == "apd" {
        "apd"
    } else {
        "ksud"
    };
    match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["--version"] => println!("{kind} fixture-1"),
        ["module", "--help"] => println!("install <zip>\nuninstall <id>\nlist"),
        ["module", "list"] => println!("[{{\"id\":\"vpnhide_core\"}}]"),
        ["module", "install", path] => {
            assert_eq!(env::var("BOOTMODE").unwrap(), "true");
            assert!(env::var("VP_HIDE_FORGED").is_err());
            let input = fs::read(path).unwrap();
            fs::write(adb.join("observed-package"), input).unwrap();
            let count = fs::read_to_string(adb.join("dispatch-count"))
                .unwrap_or_default()
                .parse::<u32>()
                .unwrap_or(0)
                + 1;
            fs::write(adb.join("dispatch-count"), count.to_string()).unwrap();
            let mode = fs::read_to_string(adb.join("mode")).unwrap_or_default();
            if mode == "fail" {
                process::exit(1)
            }
            if mode == "hang" {
                loop {
                    std::thread::park_timeout(std::time::Duration::from_secs(1));
                }
            }
            if mode == "none" {
                return;
            }
            if mode == "delay" {
                std::thread::sleep(std::time::Duration::from_millis(500));
            }
            let projection = mode.starts_with("projection");
            let id = if projection { "vpnhide_zygisk_runtime" } else { "vpnhide_core" };
            let staged = adb.join("modules_update").join(id);
            if mode == "partial" {
                fs::create_dir_all(staged).unwrap();
                return;
            }
            let template = if projection { "projection-template" } else { "replacement-template" };
            copy_tree(&adb.join(template), &staged);
            if mode == "projection-missing-abi" {
                fs::remove_file(staged.join("zygisk/armeabi-v7a.so")).unwrap();
            }
            if mode == "projection-extra" {
                fs::write(staged.join("service.sh"), b"unexpected activator").unwrap();
            }
            if mode == "projection-active" || mode == "projection-active-only" {
                copy_tree(&staged, &adb.join("modules").join(id));
                if mode == "projection-active-only" { fs::remove_dir_all(&staged).unwrap(); }
            }
            if mode == "projection-placeholder" || mode == "projection-placeholder-only" {
                let active=adb.join("modules").join(id);
                fs::create_dir_all(&active).unwrap();
                fs::copy(staged.join("module.prop"),active.join("module.prop")).unwrap();
                fs::write(active.join("update"),b"").unwrap();
                if mode == "projection-placeholder-only" { fs::remove_dir_all(&staged).unwrap(); }
            }
            if mode == "projection-forge-zip" {
                assert!(fs::write(path, b"modified input").is_err());
            }
            if mode == "projection-live-change" {
                fs::write(adb.join("modules/vpnhide_core/vpnhide-core"), b"changed").unwrap();
            }
            if mode == "live-change" {
                fs::write(adb.join("modules/vpnhide_core/vpnhide-core"), b"changed").unwrap();
            }
        }
        _ => {
            let _: PathBuf = exe;
            process::exit(2)
        }
    }
}
