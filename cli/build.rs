use std::path::Path;

fn main() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let source_dist = manifest_dir.join("../viewer/dist");
    let local_dist = manifest_dir.join("viewer-dist");

    // Copy viewer/dist/ (excluding textures) into cli/viewer-dist/ so that:
    // 1. rust-embed references a path inside the crate (required for crates.io publish)
    // 2. Textures are excluded (served separately by the texture handler)
    if source_dist.join("index.html").is_file() {
        sync_dist(&source_dist, &local_dist);
    } else if !local_dist.join("index.html").is_file() {
        // No viewer build and no previous copy — create placeholder
        std::fs::create_dir_all(&local_dist).ok();
        std::fs::write(
            local_dist.join("index.html"),
            "<!doctype html><html><body><p>Viewer not built. Run <code>cd viewer &amp;&amp; pnpm build</code> first.</p></body></html>\n",
        )
        .ok();
    }

    // Texture handling for include_bytes!
    //
    // Problem: textures live in viewer/public/textures/ (source of truth for
    // the web viewer), but include_bytes! needs a path inside the crate.
    // cargo publish tarballs only contain files under cli/, so the relative
    // path ../../../../viewer/... doesn't exist in crates.io installs.
    //
    // Solution: build.rs copies the 2K textures into cli/textures/ when
    // running in the workspace (../viewer/ exists). include_bytes! references
    // CARGO_MANIFEST_DIR/textures/ which works both in workspace builds
    // (freshly copied) and crates.io installs (bundled in tarball via
    // Cargo.toml include). cli/textures/ is gitignored to avoid duplicating
    // the files in git, but included in Cargo.toml so cargo publish picks
    // them up (same pattern as cli/viewer-dist/).
    let textures_src = manifest_dir.join("../viewer/public/textures");
    let textures_dst = manifest_dir.join("textures");
    let texture_files = [
        "earth_2k.jpg",
        "earth_night_2k.jpg",
        "moon.jpg",
        "mars.jpg",
        "sun.jpg",
    ];
    if textures_src.is_dir() {
        std::fs::create_dir_all(&textures_dst).expect("failed to create cli/textures/");
        for name in &texture_files {
            let src = textures_src.join(name);
            let dst = textures_dst.join(name);
            std::fs::copy(&src, &dst)
                .unwrap_or_else(|e| panic!("failed to copy texture {name}: {e}"));
        }
    }
    println!("cargo:rerun-if-changed=../viewer/public/textures/");

    // Rerun if viewer/dist/ changes
    println!("cargo:rerun-if-changed=../viewer/dist/");

    run_license_notice();
}

/// Workspace crates whose `wasm` feature build is packed into the viewer
/// bundle via `wasm-pack` (see `viewer/package.json` `build:wasm:*` scripts)
/// and therefore redistributed as part of the orts-cli binary. The `wasm`
/// feature switches on optional deps such as `wasm-bindgen` and
/// `serde-wasm-bindgen` that aren't reachable from the native cli graph, so
/// these need a dedicated `cargo about` pass to be covered by the notice.
/// (manifest_path, display_name, extra_cargo_about_args)
const VIEWER_WASM_CRATES: &[(&str, &str, &[&str])] = &[
    ("../arika/wasm/Cargo.toml", "arika-wasm", &[]),
    (
        "../rrd-wasm/Cargo.toml",
        "rrd-wasm",
        &["--features", "wasm", "--no-default-features"],
    ),
];

/// Generate the third-party license NOTICE via notalawyer/cargo-about.
///
/// The orts-cli binary redistributes code from two Rust dependency graphs:
///
/// 1. The native cli graph (what cargo itself compiles into the `orts`
///    binary). Covered by [`notalawyer_build::build`], which runs
///    `cargo about generate` for the current crate and writes the result
///    to `$OUT_DIR/notalawyer` for consumption via
///    `notalawyer::include_notice!()`.
/// 2. The `wasm-pack` outputs of workspace wasm crates (`arika`, `rrd-wasm`)
///    that are built with their `wasm` feature and bundled into the viewer,
///    whose compiled assets are embedded into cli via `rust-embed`. Those
///    deps are scanned by an extra `cargo about` pass per crate, written to
///    `$OUT_DIR/notalawyer_wasm_<crate>`, and concatenated by
///    [`crate::license::combined_notice`] at runtime.
///
/// Behavior:
///
/// - **docs.rs** (`DOCS_RS=1`): emit stub notices. Network and most cargo
///   plugins are not available in the docs.rs sandbox.
/// - **`cargo-about` available**: generate the real notices for every
///   redistributed graph.
/// - **`cargo-about` missing + `ORTS_REQUIRE_LICENSE_NOTICE=1`**: fail hard.
///   Our release pipeline (`.github/workflows/ci.yml` `rust-dist` job) sets
///   this env var so a misconfigured CI cannot ship a binary without a
///   valid third-party license notice.
/// - **`cargo-about` missing otherwise** (local dev, `cargo install` from
///   crates.io, one-off release builds): fall back to stub notices and
///   emit a `cargo:warning`. This keeps source-based installs working
///   without requiring contributors/installers to have `cargo-about`.
fn run_license_notice() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_dir = Path::new(&out_dir);
    let native_path = out_dir.join("notalawyer");

    // Ensure the build is re-run whenever inputs that could affect the
    // generated NOTICE change.
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=../Cargo.lock");
    println!("cargo:rerun-if-changed=about.toml");
    for (manifest, _, _) in VIEWER_WASM_CRATES {
        println!("cargo:rerun-if-changed={manifest}");
    }
    println!("cargo:rerun-if-env-changed=DOCS_RS");
    println!("cargo:rerun-if-env-changed=ORTS_REQUIRE_LICENSE_NOTICE");

    if std::env::var_os("DOCS_RS").is_some() {
        println!(
            "cargo:warning=DOCS_RS detected — embedding a placeholder \
             third-party license notice. cargo-about is not run on docs.rs \
             because its sandbox disallows network access."
        );
        write_stub_notice(
            &native_path,
            "(third-party license notice is not embedded in docs.rs builds)\n",
        );
        for (_, name, _) in VIEWER_WASM_CRATES {
            write_stub_notice(&wasm_notice_path(out_dir, name), "");
        }
        return;
    }

    let cargo_about_available = std::process::Command::new("cargo")
        .args(["about", "--version"])
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false);

    if cargo_about_available {
        // Native cli dep graph via notalawyer-build's default path.
        notalawyer_build::build();

        // Each wasm crate gets its own `cargo about` pass with the `wasm`
        // feature enabled. We share `cli/about.toml` via `-c` so the
        // accepted-license list is consistent across all passes.
        let config_path = Path::new(env!("CARGO_MANIFEST_DIR")).join("about.toml");
        let template_path =
            notalawyer_build::about_hbs().expect("failed to locate notalawyer about.hbs template");
        for (manifest, name, extra_args) in VIEWER_WASM_CRATES {
            let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest);
            let output_path = wasm_notice_path(out_dir, name);
            if let Err(err) = try_generate_wasm_notice(
                &manifest_path,
                &config_path,
                &template_path,
                &output_path,
                extra_args,
            ) {
                // A missing sibling manifest happens on packaged / single-crate
                // source installs; a non-zero cargo-about exit typically means a
                // new license needs to be accepted. Either way we only hard-fail
                // in the release pipeline where ORTS_REQUIRE_LICENSE_NOTICE=1.
                if std::env::var_os("ORTS_REQUIRE_LICENSE_NOTICE").is_some() {
                    panic!(
                        "failed to generate wasm license notice for {name} ({}): {err}",
                        manifest_path.display()
                    );
                }
                println!(
                    "cargo:warning=failed to generate wasm license notice for {name}: {err} — \
                     embedding a placeholder for this crate"
                );
                write_stub_notice(&output_path, "");
            }
        }
        return;
    }

    if std::env::var_os("ORTS_REQUIRE_LICENSE_NOTICE").is_some() {
        panic!(
            "cargo-about is required when ORTS_REQUIRE_LICENSE_NOTICE is set \
             (release-pipeline builds) so the redistributable binary embeds \
             a valid third-party license notice. Install it with \
             `cargo install cargo-about --locked` or `cargo binstall \
             cargo-about`."
        );
    }

    println!(
        "cargo:warning=cargo-about not found — embedding a placeholder \
         third-party license notice. Install cargo-about (and set \
         ORTS_REQUIRE_LICENSE_NOTICE=1 if this is a release build) to embed \
         the real notice."
    );
    write_stub_notice(
        &native_path,
        "(third-party license notice not embedded — cargo-about was not \
         available at build time; rebuild with cargo-about installed to \
         embed the real notice)\n",
    );
    for (_, name, _) in VIEWER_WASM_CRATES {
        write_stub_notice(&wasm_notice_path(out_dir, name), "");
    }
}

fn wasm_notice_path(out_dir: &Path, crate_name: &str) -> std::path::PathBuf {
    out_dir.join(format!("notalawyer_wasm_{crate_name}"))
}

fn try_generate_wasm_notice(
    manifest: &Path,
    config: &Path,
    template: &Path,
    output: &Path,
    extra_args: &[&str],
) -> Result<(), String> {
    if !manifest.exists() {
        // Happens when cli is built from a packaged crate that does not
        // carry sibling workspace crates alongside it.
        return Err(format!("manifest not found at {}", manifest.display()));
    }
    let file = std::fs::File::create(output)
        .map_err(|e| format!("failed to create wasm notice file: {e}"))?;
    let mut args = vec![
        "about",
        "generate",
        "-c",
        config
            .to_str()
            .ok_or_else(|| "about.toml path is not UTF-8".to_string())?,
        "-m",
        manifest
            .to_str()
            .ok_or_else(|| "manifest path is not UTF-8".to_string())?,
    ];
    args.extend_from_slice(extra_args);
    args.push(
        template
            .to_str()
            .ok_or_else(|| "about.hbs path is not UTF-8".to_string())?,
    );
    let status = std::process::Command::new("cargo")
        .args(&args)
        .stdout(std::process::Stdio::from(file))
        .status()
        .map_err(|e| format!("failed to spawn cargo-about: {e}"))?;
    if !status.success() {
        return Err(format!("cargo about exited with {status}"));
    }
    Ok(())
}

fn write_stub_notice(path: &Path, body: &str) {
    std::fs::write(path, body).expect("failed to write stub license notice");
}

/// Recursively copy `src` to `dst`, skipping the `textures/` directory.
fn sync_dist(src: &Path, dst: &Path) {
    // Clean previous copy to avoid stale files
    if dst.exists() {
        std::fs::remove_dir_all(dst).ok();
    }
    copy_dir_recursive(src, dst);
}

fn copy_dir_recursive(src: &Path, dst: &Path) {
    std::fs::create_dir_all(dst).ok();
    let entries = match std::fs::read_dir(src) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip textures directory — served by the texture handler
        if path.is_dir() && name_str == "textures" {
            continue;
        }

        let dest = dst.join(&name);
        if path.is_dir() {
            copy_dir_recursive(&path, &dest);
        } else {
            std::fs::copy(&path, &dest).ok();
        }
    }
}
