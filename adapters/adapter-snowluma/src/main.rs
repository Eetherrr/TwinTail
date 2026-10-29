//! SnowLuma adapter process entry point; stdout is reserved for SDK frames.
#[tokio::main(flavor = "current_thread")]
async fn main() {
    let hello = twintail_plugin_sdk::Hello {
        plugin_id: "adapter-snowluma".into(),
        version: env!("CARGO_PKG_VERSION").into(),
        required_capabilities: vec![
            "bots.register".into(),
            "events.publish".into(),
            "services.provide".into(),
        ],
    };
    if twintail_plugin_sdk::run_stdio(adapter_snowluma::SnowLuma::default(), hello)
        .await
        .is_err()
    {
        eprintln!("adapter-snowluma runtime stopped with an error");
        std::process::exit(1);
    }
}
