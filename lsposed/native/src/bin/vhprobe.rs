// Ground-truth probe: runs the exact same native checks as the in-process JNI
// path, but exec'd as root (uid 0 is not a hook target) so its view is the
// unfiltered truth. The app diffs this against its own in-process run.
//
// `--uid <n>`: the self-in-tunnel gate — report whether uid <n> is routed
// through the VPN (a policy rule steers it into a tun table). Emits
// `{uid, routed, detail}` instead of the full checks array.
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if let Some(i) = args.iter().position(|a| a == "--uid") {
        match args.get(i + 1).and_then(|s| s.parse::<u32>().ok()) {
            Some(uid) => println!("{}", vpnhide_checks::self_routed_json(uid)),
            None => {
                eprintln!("usage: vhprobe --uid <uid>");
                std::process::exit(2);
            }
        }
        return;
    }
    println!("{}", vpnhide_checks::run_all_json());
}
