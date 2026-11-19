// The version and the release repository live in one place: ../rbxpaths.py.
use std::fs;

fn pick(src: &str, key: &str) -> String {
    for line in src.lines() {
        if let Some(rest) = line.strip_prefix(&format!("{key} = \"")) {
            return rest.split('"').next().unwrap_or("").to_string();
        }
    }
    String::new()
}

fn main() {
    println!("cargo:rerun-if-changed=../rbxpaths.py");
    let src = fs::read_to_string("../rbxpaths.py").unwrap_or_default();
    println!("cargo:rustc-env=FRIDAE_VERSION={}", pick(&src, "VERSION"));
    println!("cargo:rustc-env=FRIDAE_REPO={}", pick(&src, "UPDATE_REPO"));
}
