fn main() {
    // tauri-build does not track the Windows resource icon itself. Without this,
    // incremental builds can keep an old icon embedded in the EXE.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/tray.png");
    println!("cargo:rerun-if-changed=../ui");
    tauri_build::build()
}
