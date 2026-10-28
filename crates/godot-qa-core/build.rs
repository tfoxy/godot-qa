#[allow(dead_code)]
#[path = "src/hash.rs"]
mod hash;
#[path = "src/addon_identity/manifest.rs"]
mod manifest;

use std::fmt::Write as _;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../addons/godot_qa");
    // Watching the directory recursively also detects added/deleted source files.
    println!("cargo:rerun-if-changed={}", root.display());
    println!("cargo:rerun-if-changed=src/addon_identity/manifest.rs");
    println!("cargo:rerun-if-changed=src/hash.rs");
    let entries = manifest::collect(&root).expect("read source addon identity");
    let mut generated = format!(
        "pub const EXPECTED_FINGERPRINT: &str = {:?};\n",
        manifest::fingerprint(&entries)
    );
    generated.push_str("pub const EXPECTED_FILES: &[(&str, u64, &str)] = &[\n");
    for entry in entries {
        writeln!(
            generated,
            "({:?}, {}, {:?}),",
            entry.path, entry.bytes, entry.sha256
        )
        .unwrap();
    }
    generated.push_str("];\n");
    let output = PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("addon_identity.rs");
    std::fs::write(output, generated).expect("write compiled addon identity");
}
