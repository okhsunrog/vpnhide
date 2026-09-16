mod activate;
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
        [command, mode] if command == "observe" && mode == "kpm-list" => {
            observations::kpm_list(&[])
        }
        [command, rest @ ..] if command == "mutation" && mutation::valid_command(rest) => {
            mutation::run(rest)
        }
        [command, mode] if command == "activate" && mode == "native" => activate::run(&[]),
        _ => usage_error(),
    };
    std::process::exit(code);
}

fn usage_error() -> i32 {
    eprintln!(
        "usage: vhhelper probe checks | probe routing --uid <uid> [--vpn-ifaces <list>] | observe kpm-list | mutation <directory> <config> <inspect|adopt|open|run|recover> ... | activate native"
    );
    2
}
