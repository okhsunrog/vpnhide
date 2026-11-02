//! Optional manual root native-check capture, independent of app routing.
use vpnhide_checks::run_all_json;

pub fn checks(args: &[String]) -> i32 {
    if !args.is_empty() {
        eprintln!("usage: vpnhide-core probe checks");
        return 2;
    }
    println!("{}", run_all_json());
    0
}
