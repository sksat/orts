use std::path::Path;

fn main() {
    let canonical = Path::new("../orts/wit/v0/orts.wit");
    let vendored = Path::new("wit/v0/orts.wit");

    // monorepo: canonical から自動コピー（常に同期）
    if canonical.exists() {
        println!("cargo::rerun-if-changed={}", canonical.display());
        std::fs::create_dir_all(vendored.parent().unwrap()).unwrap();
        std::fs::copy(canonical, vendored).unwrap();
    }

    // crates.io: vendored がなければエラー
    assert!(vendored.exists(), "wit/v0/orts.wit not found");
    println!("cargo::rerun-if-changed={}", vendored.display());
}
