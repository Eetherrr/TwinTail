use super::*;

/// Owned by the single maintenance handler; never locked by the reader.
#[derive(Default)]
pub(crate) struct Configuration {
    prepared: Option<ConfigSnapshot>,
}
impl Configuration {
    pub async fn handle<P: Plugin>(
        &mut self,
        plugin: &P,
        context: Context,
        method: &str,
        payload: Value,
    ) -> Result<Value> {
        match method {
            "lifecycle.started" => Ok(
                json!({"started":true,"config_revision":context.config().as_ref().map_or(0,|s|s.revision)}),
            ),
            "config.abort" => {
                let call: ConfigCommit = decode(payload)?;
                if self
                    .prepared
                    .as_ref()
                    .is_some_and(|s| s.revision != call.revision)
                {
                    return Err(error(
                        ErrorCode::Conflict,
                        "prepared configuration revision mismatch",
                    ));
                }
                self.prepared = None;
                Ok(json!({"aborted":true,"revision":call.revision}))
            }
            "config.status" => {
                let snapshot = context.config();
                Ok(
                    json!({"applied_revision":snapshot.as_ref().map(|s|s.revision),"config_version":snapshot.as_ref().map(|s|s.config_version)}),
                )
            }
            "config.prepare" => {
                let call: ConfigPrepare = decode(payload)?;
                if self.prepared.is_some() {
                    return Err(error(ErrorCode::Conflict, "configuration already prepared"));
                }
                let revision = context.config().as_ref().map_or(0, |s| s.revision);
                if call.expected_revision != revision || call.snapshot.revision <= revision {
                    return Err(error(
                        ErrorCode::Conflict,
                        "configuration revision mismatch",
                    ));
                }
                plugin
                    .prepare_config(context, call.snapshot.clone())
                    .await?;
                let revision = call.snapshot.revision;
                self.prepared = Some(call.snapshot);
                Ok(json!({"prepared":true,"revision":revision}))
            }
            "config.commit" => {
                let call: ConfigCommit = decode(payload)?;
                if self.prepared.as_ref().map(|s| s.revision) != Some(call.revision) {
                    return Err(error(
                        ErrorCode::Conflict,
                        "no matching prepared configuration",
                    ));
                }
                // Take before the callback, so both success and failure clear preparation.
                let snapshot = self.prepared.take().unwrap();
                plugin
                    .commit_config(context.clone(), snapshot.clone())
                    .await?;
                *context.0.config.lock().unwrap() = Some(snapshot);
                Ok(json!({"applied":true,"revision":call.revision}))
            }
            "host.health" => Ok(json!({"status":"ok"})),
            _ => Err(unsupported()),
        }
    }
}
