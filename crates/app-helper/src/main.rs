mod mutation;
mod probe;

fn main() {
    // Neither panic output nor errors may echo command/secret input.
    std::panic::set_hook(Box::new(|_| {}));
    let args: Vec<String> = std::env::args().skip(1).collect();
    let code = match args.as_slice() {
        [command, mode, rest @ ..] if command == "probe" && mode == "checks" => probe::run(rest),
        [command, mode, rest @ ..] if command == "probe" && mode == "routing" => probe::run(rest),
        [command, mode] if command == "observe" && mode == "kpm-list" => {
            probe::run(&["--apatch-kpm-list".to_owned()])
        }
        [command, rest @ ..] if command == "mutation" => mutation::run(rest),
        [command, mode] if command == "activate" && mode == "native" => {
            mutation::run(&["activate".to_owned(), "native".to_owned()])
        }
        // A malformed command is passed through the mature transport parser,
        // which returns its existing versioned error envelope.
        _ => mutation::run(&args),
    };
    std::process::exit(code);
}
