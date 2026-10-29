//! Regenerates the committed HTTP API contract, or checks it for drift.
fn main() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("openapi.json");
    let expected = serde_json::to_string_pretty(&web_admin::openapi()).unwrap() + "\n";
    if std::env::args().skip(1).any(|arg| arg == "--check") {
        if std::fs::read_to_string(&path).ok().as_deref() != Some(&expected) {
            eprintln!(
                "web-admin OpenAPI drift; run cargo run -p web-admin --example export_openapi"
            );
            std::process::exit(1);
        }
    } else {
        std::fs::write(path, expected).unwrap();
    }
}
