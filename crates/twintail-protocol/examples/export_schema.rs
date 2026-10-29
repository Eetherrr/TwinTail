//! Export committed wire schemas, or verify them with `--check`.

use std::{env, error::Error, fs, path::Path};

use schemars::schema_for;
use twintail_protocol::{
    ActionCall, BotRef, ConfigCommit, ConfigPrepare, ConfigSnapshot, DeliveryReport, EventEnvelope,
    Hello, Initialize, ManagementCall, Message, PublishEvent, RegisterBot, ServiceCall,
    ServiceRegistration, Subscribe,
};

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<_> = env::args().skip(1).collect();
    let check = match args.as_slice() {
        [] => false,
        [arg] if arg == "--check" => true,
        _ => return Err("usage: export_schema [--check]".into()),
    };
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../schemas");
    let schemas = [
        ("management-call-v1.json", schema_for!(ManagementCall)),
        ("protocol-v1.json", schema_for!(Message)),
        ("hello-v1.json", schema_for!(Hello)),
        ("initialize-v1.json", schema_for!(Initialize)),
        ("config-snapshot-v1.json", schema_for!(ConfigSnapshot)),
        ("config-prepare-v1.json", schema_for!(ConfigPrepare)),
        ("config-commit-v1.json", schema_for!(ConfigCommit)),
        ("bot-ref-v1.json", schema_for!(BotRef)),
        ("register-bot-v1.json", schema_for!(RegisterBot)),
        ("subscribe-v1.json", schema_for!(Subscribe)),
        ("publish-event-v1.json", schema_for!(PublishEvent)),
        ("event-envelope-v1.json", schema_for!(EventEnvelope)),
        ("delivery-report-v1.json", schema_for!(DeliveryReport)),
        ("action-call-v1.json", schema_for!(ActionCall)),
        (
            "service-registration-v1.json",
            schema_for!(ServiceRegistration),
        ),
        ("service-call-v1.json", schema_for!(ServiceCall)),
    ];
    if !check {
        fs::create_dir_all(&root)?;
    }
    for (name, schema) in schemas {
        let expected = format!("{}\n", serde_json::to_string_pretty(&schema)?);
        let path = root.join(name);
        if check {
            if fs::read_to_string(&path)? != expected {
                return Err(
                    format!("schema drift: {}; rerun export_schema", path.display()).into(),
                );
            }
        } else {
            fs::write(path, expected)?;
        }
    }
    Ok(())
}
