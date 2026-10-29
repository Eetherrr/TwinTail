//! Embeds the committed frontend build without requiring Node at Rust build time.
use std::{
    env, fs,
    path::{Component, Path, PathBuf},
};

/// Converts a relative filesystem path to a platform-independent asset URL.
fn asset_url(relative: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in relative.components() {
        let Component::Normal(name) = component else {
            return None;
        };
        let name = name.to_str()?;
        if name.is_empty() || name.contains(['/', '\\']) || name.chars().any(char::is_control) {
            return None;
        }
        parts.push(name);
    }
    if parts.is_empty() {
        return None;
    }
    Some(format!("/{}", parts.join("/")))
}

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
            asset_url(path.strip_prefix(root).unwrap())
                .expect("web assets must have safe relative UTF-8 paths");
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
        let url = asset_url(path.strip_prefix(&root).unwrap())
            .expect("web assets must have safe relative UTF-8 paths");
        generated.push_str(&format!(
            "({:?},include_bytes!({:?})),\n",
            url,
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

#[cfg(test)]
mod tests {
    use super::asset_url;
    use std::path::Path;

    #[test]
    fn native_relative_components_produce_url_separators() {
        let path = Path::new("assets").join("nested").join("app.js");
        assert_eq!(asset_url(&path).as_deref(), Some("/assets/nested/app.js"));
        assert_eq!(
            asset_url(Path::new("index.html")).as_deref(),
            Some("/index.html")
        );
    }

    #[test]
    fn unsafe_relative_components_are_rejected() {
        for path in [
            "",
            "..",
            "../index.html",
            "assets/../index.html",
            "./index.html",
            "/index.html",
            "assets/line\n.js",
            "assets/null\0.js",
        ] {
            assert_eq!(asset_url(Path::new(path)), None, "{path:?}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn windows_separators_are_accepted_and_prefixes_are_rejected() {
        assert_eq!(
            asset_url(Path::new(r"assets\nested\app.js")).as_deref(),
            Some("/assets/nested/app.js")
        );
        for path in [
            r"C:\assets\app.js",
            r"C:assets\app.js",
            r"\assets\app.js",
            r"\\server\share\app.js",
            r"\\?\C:\assets\app.js",
            r"assets\..\app.js",
        ] {
            assert_eq!(asset_url(Path::new(path)), None, "{path:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_and_literal_backslash_names_are_rejected() {
        use std::{ffi::OsStr, os::unix::ffi::OsStrExt};
        assert_eq!(
            asset_url(Path::new(OsStr::from_bytes(b"assets/\xff.js"))),
            None
        );
        assert_eq!(asset_url(Path::new(r"assets\app.js")), None);
    }
}
