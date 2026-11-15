fn main() {
    // tauri-build does not track the Windows resource icon itself. Without this,
    // incremental builds can keep an old icon embedded in the EXE.
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/icon.png");
    println!("cargo:rerun-if-changed=icons/tray.png");
    println!("cargo:rerun-if-changed=../ui");
    println!("cargo:rerun-if-changed=../LICENSE");
    let root = std::path::PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap());
    let license = std::fs::read(root.join("../LICENSE")).expect("read application MIT license");
    let target = root.join("resources/Voice-Input-LICENSE.txt");
    if std::fs::read(&target).ok().as_deref() != Some(license.as_slice()) {
        std::fs::write(target, license).expect("stage application MIT license");
    }
    tauri_build::build()
}
