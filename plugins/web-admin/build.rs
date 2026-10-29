//! Embeds the committed frontend build without requiring Node at Rust build time.
use std::{
    env, fs,
    path::{Path, PathBuf},
};
fn collect(root: &Path, current: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = fs::read_dir(current)
        .expect("read tracked web assets")
        .map(|e| e.unwrap().path())
        .collect();
    entries.sort();
    for path in entries {
        let meta = fs::symlink_metadata(&path).unwrap();
        assert!(
            !meta.file_type().is_symlink(),
            "web assets must not be symlinks"
        );
        if meta.is_dir() {
            collect(root, &path, out);
        } else {
            assert!(meta.is_file(), "web asset must be regular");
            let relative = path.strip_prefix(root).unwrap();
            assert!(!relative.to_string_lossy().contains('\\'));
            out.push(path);
        }
    }
}
fn main() {
    let root = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap()).join("assets");
    println!("cargo:rerun-if-changed=assets");
    assert!(
        root.join("index.html").is_file(),
        "tracked web assets missing; run npm ci && npm run build in web/"
    );
    let mut files = Vec::new();
    collect(&root, &root, &mut files);
    let mut generated = String::from("static ASSETS:&[(&str,&[u8])]=&[\n");
    for path in files {
        let relative = path
            .strip_prefix(&root)
            .unwrap()
            .to_str()
            .unwrap()
            .replace('\\', "/");
        generated.push_str(&format!(
            "({:?},include_bytes!({:?})),\n",
            format!("/{relative}"),
            path.to_str().unwrap()
        ));
    }
    generated.push_str("];\n");
    fs::write(
        PathBuf::from(env::var("OUT_DIR").unwrap()).join("assets.rs"),
        generated,
    )
    .unwrap();
}
