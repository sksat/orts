use std::path::Path;

fn main() {
    // rust-embed requires the folder to exist at compile time.
    // Create a minimal viewer/dist/ with a placeholder index.html
    // when the viewer hasn't been built yet.
    let dist = Path::new(env!("CARGO_MANIFEST_DIR")).join("../viewer/dist");
    if !dist.join("index.html").is_file() {
        std::fs::create_dir_all(&dist).ok();
        std::fs::write(
            dist.join("index.html"),
            "<!doctype html><html><body><p>Viewer not built. Run <code>cd viewer &amp;&amp; pnpm build</code> first.</p></body></html>\n",
        )
        .ok();
    }
}
