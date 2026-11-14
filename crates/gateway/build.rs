use sha2::{Digest, Sha256};
use std::{
    env, fs,
    path::{Path, PathBuf},
};

fn files(root: &Path, path: &Path, output: &mut Vec<PathBuf>) -> std::io::Result<()> {
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        let kind = entry.file_type()?;
        if kind.is_dir() {
            files(root, &entry.path(), output)?;
        } else if kind.is_file() {
            output.push(entry.path().strip_prefix(root).expect("child path").into());
        } else {
            return Err(std::io::Error::other("UI inputs must be regular files"));
        }
    }
    Ok(())
}

fn fingerprint(root: &Path, paths: &mut [PathBuf]) -> std::io::Result<String> {
    paths.sort();
    let mut hash = Sha256::new();
    for path in paths {
        let content = Sha256::digest(fs::read(root.join(&*path))?);
        hash.update(format!("{}\0{content:x}\n", path.display()));
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn main() -> std::io::Result<()> {
    let krate = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").expect("Cargo manifest"));
    let ui = krate.join("../../ui");
    let staged = krate.join("ui-dist");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed={}", staged.display());
    let mut inputs = Vec::new();
    for name in [
        "src",
        "public",
        "scripts",
        "index.html",
        "vite.config.ts",
        "tsconfig.json",
        "components.json",
        "package.json",
        "bun.lock",
    ] {
        let path = ui.join(name);
        println!("cargo:rerun-if-changed={}", path.display());
        if path.is_dir() {
            files(&ui, &path, &mut inputs)?;
        } else if path.is_file() {
            inputs.push(name.into());
        }
    }
    let mut assets = Vec::new();
    files(&staged, &staged, &mut assets)?;
    assets.sort();
    let current = fingerprint(&ui, &mut inputs)?;
    let output = fingerprint(&staged, &mut assets)?;
    let expected = format!("{current}\n{output}\n");
    let recorded = fs::read_to_string(staged.join(".fingerprint")).unwrap_or_default();
    if recorded != expected || !staged.join("index.html").is_file() {
        println!(
            "cargo:warning=Embedded UI is missing, stale or modified; run bun run --cwd ui stage before building a release artifact."
        );
    } else {
        println!("cargo:warning=Embedded UI {}", &current[..12]);
    }
    let mut generated = String::from("static ASSETS: &[Asset] = &[\n");
    for path in assets {
        let relative = format!("/{}", path.display());
        let absolute = staged.join(path).canonicalize()?;
        generated.push_str(&format!(
            "Asset {{ path: {relative:?}, bytes: include_bytes!({absolute:?}) }},\n"
        ));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var_os("OUT_DIR").expect("Cargo output")).join("ui_assets.rs"),
        generated,
    )
}
