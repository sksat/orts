use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    // Help cc crate find WASI sysroot for wasm32-wasip1 cross-compilation
    if std::env::var("WASI_SYSROOT").is_err() {
        let default_sysroot = Path::new("/usr/share/wasi-sysroot");
        if default_sysroot.exists() {
            // SAFETY: build script is single-threaded
            unsafe { std::env::set_var("WASI_SYSROOT", default_sysroot) };
        }
    }

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let src_dir = resolve_source_dir(&out_dir);

    let shared_dir = src_dir.join("fsw").join("shared");
    let cfs_src_dir = src_dir.join("fsw").join("cfs").join("src");

    // Verify required files exist
    let adac_c = shared_dir.join("generic_adcs_adac.c");
    let util_c = shared_dir.join("generic_adcs_utilities.c");
    let msg_h = cfs_src_dir.join("generic_adcs_msg.h");
    for f in [&adac_c, &util_c, &msg_h] {
        assert!(f.exists(), "Required file not found: {}", f.display());
    }

    // Build the standalone msg.h shim that replaces cFS-dependent types
    let shim_dir = out_dir.join("shim");
    std::fs::create_dir_all(&shim_dir).unwrap();
    write_msg_shim(&shim_dir, &msg_h);

    // Compile C sources to static library
    cc::Build::new()
        .file(&adac_c)
        .file(&util_c)
        .include(&shim_dir) // shim's generic_adcs_msg.h shadows the cFS one
        .include(&shared_dir)
        .define("ACS_IN_FSW", None) // suppress printf in UNITQ
        .warnings(false)
        .compile("generic_adcs");

    // rerun-if-changed
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", adac_c.display());
    println!("cargo:rerun-if-changed={}", util_c.display());
}

/// Resolve the generic_adcs source directory.
///
/// Priority:
/// 1. `GENERIC_ADCS_SRC_DIR` environment variable (for offline / custom builds)
/// 2. Git clone into `OUT_DIR/generic_adcs` using metadata from Cargo.toml
fn resolve_source_dir(out_dir: &Path) -> PathBuf {
    if let Ok(dir) = std::env::var("GENERIC_ADCS_SRC_DIR") {
        let p = PathBuf::from(dir);
        assert!(p.exists(), "GENERIC_ADCS_SRC_DIR does not exist: {}", p.display());
        return p;
    }

    // Read repository and commit from Cargo.toml metadata
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap();
    let cargo_toml = std::fs::read_to_string(Path::new(&manifest).join("Cargo.toml")).unwrap();
    let repo = extract_toml_value(&cargo_toml, "package.metadata.generic-adcs", "repository")
        .expect("missing [package.metadata.generic-adcs] repository");
    let commit = extract_toml_value(&cargo_toml, "package.metadata.generic-adcs", "commit")
        .expect("missing [package.metadata.generic-adcs] commit");

    let clone_dir = out_dir.join("generic_adcs");
    git_clone_or_fetch(&clone_dir, &repo, &commit);
    clone_dir
}

fn git_clone_or_fetch(dir: &Path, repo: &str, commit: &str) {
    if dir.join(".git").exists() {
        // Already cloned — check if correct commit
        let current = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(dir)
            .output()
            .expect("git rev-parse failed");
        let current_hash = String::from_utf8_lossy(&current.stdout).trim().to_string();
        if current_hash == commit {
            return; // Already at correct commit
        }
        // Fetch and checkout
        let status = Command::new("git")
            .args(["fetch", "origin"])
            .current_dir(dir)
            .status()
            .expect("git fetch failed");
        assert!(status.success(), "git fetch failed");
    } else {
        // Fresh clone
        let status = Command::new("git")
            .args(["clone", repo, &dir.to_string_lossy()])
            .status()
            .expect("git clone failed");
        assert!(status.success(), "git clone failed");
    }

    // Checkout specific commit
    let status = Command::new("git")
        .args(["checkout", commit])
        .current_dir(dir)
        .status()
        .expect("git checkout failed");
    assert!(status.success(), "git checkout {} failed", commit);
}

/// Write a standalone generic_adcs_msg.h that replaces the cFS-dependent original.
/// We extract only the Payload types (no CFE_MSG_* headers).
fn write_msg_shim(shim_dir: &Path, original_msg_h: &Path) {
    let content = std::fs::read_to_string(original_msg_h).unwrap();

    let mut shim = String::from(
        "/* Auto-generated standalone shim — removes cFS dependencies */\n\
         #ifndef _GENERIC_ADCS_MSG_H_\n\
         #define _GENERIC_ADCS_MSG_H_\n\n\
         #include <stdint.h>\n\
         #include <stdbool.h>\n\n",
    );

    // Extract all typedef struct blocks that end with _Payload_t or _Tlm_t or _Hmgmt_t
    // We need the payload types but not the wrapper types with CFE headers
    let mut in_typedef = false;
    let mut brace_depth: i32 = 0;
    let mut current_block = String::new();

    for line in content.lines() {
        if line.starts_with("typedef struct") {
            in_typedef = true;
            brace_depth = 0;
            current_block.clear();
        }

        if in_typedef {
            // Replace `uint8` (cFS typedef) with `uint8_t` (standard C)
            let fixed = line.replace("uint8 ", "uint8_t ");
            current_block.push_str(&fixed);
            current_block.push('\n');

            brace_depth += line.chars().filter(|&c| c == '{').count() as i32;
            brace_depth -= line.chars().filter(|&c| c == '}').count() as i32;

            if brace_depth == 0 && line.contains('}') {
                in_typedef = false;
                // Include only types that don't reference CFE_MSG_*
                if !current_block.contains("CFE_MSG_") {
                    shim.push_str(&current_block);
                    shim.push('\n');
                }
            }
        }

        // Also include #define lines for command codes and mode constants
        if line.starts_with("#define GENERIC_ADCS_") && !line.contains("LNGTH") {
            shim.push_str(line);
            shim.push('\n');
        }
    }

    shim.push_str("#endif\n");

    std::fs::write(shim_dir.join("generic_adcs_msg.h"), shim).unwrap();
}

/// Simple TOML value extractor for dotted keys like "package.metadata.generic-adcs.repository"
fn extract_toml_value(toml_content: &str, section: &str, key: &str) -> Option<String> {
    let section_header = format!("[{}]", section);
    let mut in_section = false;

    for line in toml_content.lines() {
        let trimmed = line.trim();
        if trimmed == section_header {
            in_section = true;
            continue;
        }
        if in_section && trimmed.starts_with('[') {
            break; // Next section
        }
        if in_section {
            if let Some(rest) = trimmed.strip_prefix(&format!("{} = ", key)) {
                return Some(rest.trim_matches('"').to_string());
            }
            if let Some(rest) = trimmed.strip_prefix(&format!("{}=", key)) {
                return Some(rest.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}
