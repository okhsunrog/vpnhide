mod core_client;
mod mutation;
mod observations;
mod probe;

fn main() {
    // Neither panic output nor errors may echo command/secret input.
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.as_slice() {
        [command, mode] if command == "probe" && mode == "checks" => probe::checks(&[]),
        [command, mode, rest @ ..] if command == "probe" && mode == "routing" => {
            probe::routing(rest)
        }
        [command, mode, rest @ ..] if command == "observe" && mode == "app-vpn-state" => {
            probe::app_vpn_state(rest)
        }
        [command, mode] if command == "observe" && mode == "kpm-list" => {
            observations::kpm_list(&[])
        }
        [command, rest @ ..] if command == "core" && core_client::valid_command(rest) => {
            core_client::run(rest)
        }
        [command, rest @ ..] if command == "mutation" && mutation::valid_command(rest) => {
            mutation::run(rest)
        }
        _ => usage_error(),
    };
    std::process::exit(code);
}

fn usage_error() -> i32 {
    eprintln!(
        "usage: vhhelper probe checks | probe routing --uid <uid> [--vpn-ifaces <list>] | observe app-vpn-state --uid <uid> | observe kpm-list | core <state | operation <op> | replace <op> <base> | repair <op> <base>> | mutation <directory> <config> <inspect|adopt|open|run|recover> ..."
    );
    2
}
