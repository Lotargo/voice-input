#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    voice_input_lib::configure_native_runtime();
    if let Some(code) = voice_input_lib::run_helper_if_requested() {
        std::process::exit(code);
    }
    voice_input_lib::run();
}
