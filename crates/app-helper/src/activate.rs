//! Companion activation entry point. Activation is intentionally separate from
//! mutation transport; the coordinator still supervises it as a child when the
//! `activate native` command is used.

pub fn run(args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("usage: vhhelper activate native");
        return 2;
    }
    match vpnhide_activator::activate_native_companion() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("native activation refused: {error}");
            1
        }
    }
}
