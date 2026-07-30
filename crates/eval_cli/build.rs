fn main() {
    let cargo_toml = std::fs::read_to_string("../omega/Cargo.toml")
        .expect("Failed to read crates/omega/Cargo.toml");
    let version = cargo_toml
        .lines()
        .find(|line| line.starts_with("version = "))
        .expect("Version not found in crates/omega/Cargo.toml")
        .split('=')
        .nth(1)
        .expect("Invalid version format")
        .trim()
        .trim_matches('"');
    println!("cargo:rerun-if-changed=../omega/Cargo.toml");
    println!("cargo:rustc-env=ZED_PKG_VERSION={}", version);
}
