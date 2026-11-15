use voice_input_lib::cli::{request, Request};
fn main() {
    let mut args = std::env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "--help".into());
    if command == "--help" || command == "-h" {
        println!("voice-input-cli <status|start|stop|models|select-model ID|records|new-record|clear-record|mic-test|show|hide|quit>\nJSON on stdout. Exit codes: 0 success, 1 command/transport error, 2 usage.\nStart the GUI first. start/mic-test acknowledge launch; poll status for readiness.\nstop waits for native model destruction. quit waits before exiting.\nCommands are never retried automatically; after timeout the outcome is unknown.");
        return;
    }
    if command == "--version" {
        println!("{}", env!("CARGO_PKG_VERSION"));
        return;
    }
    let argument = args.next();
    if args.next().is_some() {
        eprintln!("Too many arguments; run --help");
        std::process::exit(2);
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let response = match runtime.block_on(request(Request { command, argument })) {
        Ok(value) => value,
        Err(error) => serde_json::json!({"ok":false,"error":error}),
    };
    println!("{response}");
    if response["ok"] != true {
        std::process::exit(1);
    }
}
