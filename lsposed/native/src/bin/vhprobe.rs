// Ground-truth probe: runs the exact same native checks as the in-process JNI
// path, but exec'd as root (uid 0 is not a hook target) so its view is the
// unfiltered truth. The app diffs this against its own in-process run.
fn main() {
    println!("{}", vpnhide_checks::run_all_json());
}
