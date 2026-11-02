use std::{env, fs, path::PathBuf};
fn main() {
    println!("cargo:rerun-if-env-changed=VPNHIDE_CORE_TRUST_FILE");
    println!("cargo:rerun-if-env-changed=VPNHIDE_BUILD_SEQUENCE");
    let sequence = match env::var("VPNHIDE_BUILD_SEQUENCE") {
        Ok(value) => value.parse::<u64>().expect("invalid signed build sequence"),
        Err(_) => 0,
    };
    let text = match env::var_os("VPNHIDE_CORE_TRUST_FILE") {
        Some(path) => {
            println!("cargo:rerun-if-changed={}", PathBuf::from(&path).display());
            let bytes = fs::read(path).expect("cannot read public Core trust configuration");
            assert!(
                bytes.len() <= 16 * 1024,
                "Core trust configuration exceeds limit"
            );
            String::from_utf8(bytes).expect("Core trust configuration is not UTF-8")
        }
        None => String::from("{\"schema\":1,\"keys\":[]}"),
    };
    let out = PathBuf::from(env::var_os("OUT_DIR").expect("OUT_DIR missing"));
    fs::write(
        out.join("trust.rs"),
        format!("const COMPILED_TRUST_JSON: &str = {text:?};\npub const COMPILED_BUILD_SEQUENCE: u64 = {sequence};\n"),
    )
    .expect("cannot write public Core trust configuration");
}
