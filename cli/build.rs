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
/// these need a dedicated license pass (with that feature enabled) to be
/// covered by the notice.
/// (manifest_path, display_name, no_default_features, cargo_features)
const VIEWER_WASM_CRATES: &[(&str, &str, bool, &[&str])] = &[
    ("../arika/wasm/Cargo.toml", "arika-wasm", false, &[]),
    ("../rrd-wasm/Cargo.toml", "rrd-wasm", true, &["wasm"]),
];

/// Generate the third-party license NOTICE via notalawyer/cargo-about.
///
/// The orts-cli binary redistributes code from two Rust dependency graphs:
///
/// 1. The native cli graph (what cargo itself compiles into the `orts`
///    binary). Covered by [`notalawyer_build::build`], which gathers the
///    current crate's licenses via the `cargo-about` library and writes the
///    result to `$OUT_DIR/notalawyer` for consumption via
///    `notalawyer::include_notice!()`.
/// 2. The `wasm-pack` outputs of workspace wasm crates (`arika`, `rrd-wasm`)
///    that are built with their `wasm` feature and bundled into the viewer,
///    whose compiled assets are embedded into cli via `rust-embed`. Those
///    deps are gathered by [`gather_wasm_notice`] (the cargo-about library,
///    pointed at each wasm manifest with its `wasm` feature), written to
///    `$OUT_DIR/notalawyer_wasm_<crate>`, and concatenated by
///    [`crate::license::combined_notice`] at runtime.
///
/// Both passes use the cargo-about *library* (no `cargo about` binary is
/// required), so a stale/missing binary can no longer silently skip the notice.
///
/// Behavior:
///
/// - **docs.rs** (`DOCS_RS=1`): emit stub notices. The docs.rs sandbox
///   disallows network access, so we don't run the gatherer there.
/// - **otherwise**: generate the real notices. The native pass
///   ([`notalawyer_build::build`]) panics on failure. A wasm pass that fails —
///   most commonly because the sibling manifest is absent (a packaged /
///   single-crate source install only carries `cli/`) — falls back to a stub
///   and a `cargo:warning`, unless `ORTS_REQUIRE_LICENSE_NOTICE=1` (set by the
///   `.github/workflows/ci.yml` `rust-dist` release job), in which case it
///   fails hard so a release binary can't ship an incomplete notice.
fn run_license_notice() {
    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let out_dir = Path::new(&out_dir);
    let native_path = out_dir.join("notalawyer");

    // Ensure the build is re-run whenever inputs that could affect the
    // generated NOTICE change.
    println!("cargo:rerun-if-changed=Cargo.toml");
    println!("cargo:rerun-if-changed=../Cargo.lock");
    println!("cargo:rerun-if-changed=about.toml");
    for (manifest, _, _, _) in VIEWER_WASM_CRATES {
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
        write_notice(
            &native_path,
            "(third-party license notice is not embedded in docs.rs builds)\n",
        );
        for (_, name, _, _) in VIEWER_WASM_CRATES {
            write_notice(&wasm_notice_path(out_dir, name), "");
        }
        return;
    }

    // Native cli dep graph via notalawyer-build's default path (cargo-about
    // library; no `cargo about` binary needed).
    notalawyer_build::build();

    // Each bundled wasm crate is gathered separately, pointed at its own
    // manifest with the `wasm` feature, because notalawyer-build 0.3's
    // gather() only covers the current crate (arkedge/notalawyer#32). The
    // shared `cli/about.toml` keeps the accepted-license list / ignores
    // consistent with the native pass, and gather_wasm_notice renders in the
    // same format so the per-crate notices concatenate cleanly at runtime.
    let require = std::env::var_os("ORTS_REQUIRE_LICENSE_NOTICE").is_some();
    for (manifest, name, no_default_features, features) in VIEWER_WASM_CRATES {
        let manifest_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(manifest);
        let output_path = wasm_notice_path(out_dir, name);
        match gather_wasm_notice(&manifest_path, *no_default_features, features) {
            Ok(notice) => write_notice(&output_path, &notice),
            // A missing sibling manifest happens on packaged / single-crate
            // source installs (only `cli/` is shipped). Hard-fail only in the
            // release pipeline; otherwise embed a placeholder for this crate.
            Err(err) if require => panic!(
                "failed to generate wasm license notice for {name} ({}): {err}",
                manifest_path.display()
            ),
            Err(err) => {
                println!(
                    "cargo:warning=failed to generate wasm license notice for {name}: {err} — \
                     embedding a placeholder for this crate"
                );
                write_notice(&output_path, "");
            }
        }
    }
}

/// Gather the third-party license notice for a single (non-current) crate via
/// the cargo-about library, rendered in the same format as
/// [`notalawyer_build::build`] so per-crate notices concatenate cleanly.
///
/// This mirrors what `notalawyer-build` does internally, but parameterized by
/// `manifest` + feature selection (which its public `gather()` can't express).
/// `cli/about.toml` is loaded explicitly so the accepted-license list and
/// `ignore-build-dependencies` / `ignore-dev-dependencies` match the native
/// pass. Under `ORTS_REQUIRE_LICENSE_NOTICE=1` an unresolvable license text is
/// an error rather than a warning, so the release pipeline cannot ship a notice
/// that silently omits a crate. No remote *license-file* fetching happens — the config has no
/// `clarify.git` entries, so the gatherer is given no HTTP client. (Crate
/// resolution still uses Cargo's normal index access.) See
/// arkedge/notalawyer#32 for folding this back into notalawyer-build.
fn gather_wasm_notice(
    manifest: &Path,
    no_default_features: bool,
    features: &[&str],
) -> Result<String, String> {
    use cargo_about::licenses::config::{Config, KrateConfig};
    use std::collections::BTreeMap;
    use std::fmt::Write as _;
    use std::sync::Arc;
    use toml_span::Deserialize as _;

    if !manifest.exists() {
        return Err(format!("manifest not found at {}", manifest.display()));
    }
    let manifest_path = camino::Utf8PathBuf::from_path_buf(manifest.to_path_buf())
        .map_err(|_| "manifest path is not UTF-8".to_string())?;

    let about_toml = Path::new(env!("CARGO_MANIFEST_DIR")).join("about.toml");
    let contents = std::fs::read_to_string(&about_toml)
        .map_err(|e| format!("failed to read {}: {e}", about_toml.display()))?;
    let mut value =
        toml_span::parse(&contents).map_err(|e| format!("failed to parse about.toml: {e}"))?;
    let cfg = Config::deserialize(&mut value)
        .map_err(|e| format!("failed to deserialize about.toml: {e:?}"))?;

    let krates = cargo_about::get_all_crates(
        &manifest_path,
        no_default_features,
        false, // all_features
        features.iter().map(|s| (*s).to_string()).collect(),
        false, // workspace
        krates::LockOptions {
            frozen: false,
            locked: false,
            offline: false,
        },
        &cfg,
        &[],
    )
    .map_err(|e| format!("failed to resolve crates: {e}"))?;

    let store =
        cargo_about::licenses::store_from_cache().map_err(|e| format!("license store: {e}"))?;

    // No HTTP client: `cli/about.toml` has no `clarify.git` entries, so the
    // gatherer never needs to fetch remote license files.
    let summary = cargo_about::licenses::Gatherer::with_store(Arc::new(store))
        .with_confidence_threshold(0.8)
        .with_max_depth(cfg.max_depth.map(|md| md as _))
        .gather(&krates, &cfg, None);

    let krate_cfg: BTreeMap<String, KrateConfig> = cfg
        .crates
        .into_iter()
        .map(|(name, spanned)| (name, spanned.value))
        .collect();

    // In the release pipeline a crate whose license text cannot be resolved must
    // fail the build, not quietly drop out of the notice: with `fail_on_missing`
    // the "no `license` and no license files" case becomes an Error-severity
    // diagnostic, which `generate` below refuses to render. Outside the release
    // pipeline it stays a warning so a normal `cargo build` is not held hostage
    // to a dependency's packaging. This mirrors how the caller treats our Err.
    let fail_on_missing = std::env::var_os("ORTS_REQUIRE_LICENSE_NOTICE").is_some();

    let mut files = cargo_about::licenses::resolution::Files::new();
    let resolved = cargo_about::licenses::resolution::resolve(
        &summary,
        &cfg.accepted,
        &krate_cfg,
        &mut files,
        fail_on_missing,
    );

    use codespan_reporting::term;
    let stream = term::termcolor::StandardStream::stderr(term::termcolor::ColorChoice::Auto);
    let diag_cfg = term::Config::default();
    let license_list = cargo_about::generate::generate(&summary, &resolved, |diags| {
        let mut stream = stream.lock();
        for diag in diags {
            let _ = term::emit_to_io_write(&mut stream, &diag_cfg, &files, diag);
        }
    })
    .map_err(|e| format!("failed to generate license list: {e}"))?;

    // Same layout as notalawyer-build's built-in default renderer, so the
    // wasm notices match the native pass byte-for-byte.
    let dashes = "-".repeat(74);
    let mut out = String::new();
    for license in &license_list.licenses {
        writeln!(out, "{}\n Used by:", license.name).unwrap();
        for used_by in &license.used_by {
            let krate = used_by.krate;
            let link = match krate.repository.as_deref() {
                Some(repo) => format!(" {repo} "),
                None => format!(" https://crates.io/crates/{} ", krate.name),
            };
            writeln!(out, "  - {} {} ({link})", krate.name, krate.version).unwrap();
        }
        writeln!(out, "\n{}\n{dashes}", license.text).unwrap();
    }
    Ok(out)
}

fn wasm_notice_path(out_dir: &Path, crate_name: &str) -> std::path::PathBuf {
    out_dir.join(format!("notalawyer_wasm_{crate_name}"))
}

fn write_notice(path: &Path, body: &str) {
    std::fs::write(path, body).expect("failed to write license notice");
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
