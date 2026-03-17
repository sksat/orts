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

    // Rerun if viewer/dist/ changes
    println!("cargo:rerun-if-changed=../viewer/dist/");
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
