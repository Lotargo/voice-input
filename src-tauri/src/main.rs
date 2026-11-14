#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Some(code) = voice_input_lib::run_helper_if_requested() {
        std::process::exit(code);
    }
    voice_input_lib::run();
}
