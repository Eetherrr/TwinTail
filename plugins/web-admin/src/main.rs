//! Web administrator IPC entrypoint; stdout is exclusively SDK frames.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let hello = twintail_plugin_sdk::Hello {
        plugin_id: "web-admin".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        required_capabilities: vec!["management.read".into()],
    };
    if twintail_plugin_sdk::run_stdio(web_admin::WebAdmin::default(), hello)
        .await
        .is_err()
    {
        eprintln!("web-admin runtime stopped with an error");
        std::process::exit(1);
    }
}
