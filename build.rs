fn main() {
    embuild::espidf::sysenv::output();

    // Pass custom linker script to discard defmt sections (from lora-phy dependency)
    println!(
        "cargo:rustc-link-arg=-T{}/defmt-discard.x",
        std::env::var("CARGO_MANIFEST_DIR").unwrap()
    );

    // This build's version, for firmware.rs VERSION: the git hash, -dirty
    // with uncommitted changes. Worked out again whenever the code or git's
    // state changes (ESP-IDF's own version, esp_app_desc, is only worked
    // out when its CMake step reruns, which Rust changes don't make it do)
    let version = std::process::Command::new("git")
        .args(["describe", "--always", "--dirty"])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
        .unwrap_or_else(|| "unknown".into());
    println!("cargo:rustc-env=OSWST_VERSION={}", version);
    for path in ["src", "core/src", ".git/HEAD", ".git/index", ".git/refs"] {
        println!("cargo:rerun-if-changed={}", path);
    }
}
