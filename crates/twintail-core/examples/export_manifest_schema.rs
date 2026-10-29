//! Export manifest and host configuration schemas, or verify them with `--check`.

use std::{error::Error, fs, path::PathBuf};
use twintail_core::{HostConfig, Manifest};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let check = match args.as_slice() {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err("usage: export_manifest_schema [--check]".into()),
    };
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let schemas = [
        ("plugin-manifest-v1.json", schemars::schema_for!(Manifest)),
        ("host-config-v1.json", schemars::schema_for!(HostConfig)),
    ];
    for (name, schema) in schemas {
        let content = format!("{}\n", serde_json::to_string_pretty(&schema)?);
        let path = root.join(name);
        if check {
            if fs::read_to_string(&path)? != content {
                return Err(format!("schema drift: {}", path.display()).into());
            }
        } else {
            fs::create_dir_all(&root)?;
            fs::write(&path, content)?;
        }
    }
    Ok(())
}
