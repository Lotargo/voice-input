use std::path::Path;
fn main() {
    voice_input_lib::configure_native_runtime();
    let args = std::env::args().collect::<Vec<_>>();
    let result = match args.get(1).map(String::as_str) {
        Some("download") if args.len() >= 4 => voice_input_lib::model_tools::download(&args[2], Path::new(&args[3]), args.get(4).map(|v| v.parse().expect("crash byte count"))),
        Some("transcribe") if (5..=7).contains(&args.len()) => voice_input_lib::model_tools::transcribe_with_threads(&args[2], Path::new(&args[3]), Path::new(&args[4]), args.get(5).map(|v| v.parse().expect("CPU threads")), args.get(6).is_some_and(|v| v == "no-spin")).map(|result| println!("{result}")),
        _ => Err("model-check download ID QA_DIRECTORY [CRASH_AFTER_BYTES] | transcribe ID QA_DIRECTORY WAV_OR_PCM [THREADS]".into()),
    };
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
