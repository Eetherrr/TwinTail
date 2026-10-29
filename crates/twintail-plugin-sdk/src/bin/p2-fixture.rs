//! Internal integration-test executable reusing the real example implementations.
//! This binary and its modes are not part of the public SDK API.
#[path = "../../../../examples/echo-plugin/src/main.rs"]
mod echo;
#[path = "../../../../examples/mock-adapter/src/main.rs"]
mod mock;
fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("echo") => echo::main(),
        Some("mock") => mock::main(),
        _ => {
            eprintln!("usage: p2-fixture echo|mock");
            std::process::exit(2);
        }
    }
}
