//! Versioned machine-readable HTTP contract.
use serde_json::{Value, json};
/// OpenAPI 3.1 document for the loopback web administrator HTTP boundary.
/// Export with `cargo run -p web-admin --example export_openapi`.
pub fn openapi() -> Value {
    let error = json!({"description":"Safe error; never contains upstream message/details or credentials","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Error"}}}});
    let mut failures = serde_json::Map::new();
    for status in [
        "400", "401", "403", "404", "409", "413", "415", "429", "501", "502", "504",
    ] {
        failures.insert(status.into(), error.clone());
    }
    let mut session_responses = failures.clone();
    session_responses.insert("200".into(),json!({"description":"Session and CSRF bootstrap; login also sets HttpOnly SameSite=Strict cookie","content":{"application/json":{"schema":{"$ref":"#/components/schemas/Session"}}}}));
    let mut logout_responses = failures.clone();
    logout_responses.insert(
        "204".into(),
        json!({"description":"Session revoked and cookie cleared"}),
    );
    let mut manage_responses = failures;
    manage_responses.insert("200".into(),json!({"description":"Host result; mutations acknowledge an operation and must be polled","content":{"application/json":{"schema":{"type":"object","required":["result"],"properties":{"result":{}}}}}}));
    let csrf = json!({"name":"X-CSRF-Token","in":"header","required":true,"schema":{"type":"string","pattern":"^[0-9a-f]{64}$"}});
    let origin = json!({"name":"Origin","in":"header","required":true,"schema":{"type":"string"},"description":"Exact http://bound-loopback-IP:port; forwarded origins and localhost aliases rejected"});
    let mut api = json!({"openapi":"3.1.0","info":{"title":"TwinTail loopback web administrator","version":"1.0.0","description":"All requests require exact Host and no query string. Mutations require exact Origin; GET with an Origin also requires exact match. Sessions use fixed TTL. All manage calls, including reads, require CSRF. No CORS. Request body/time/concurrency/connection/session limits apply."},"paths":{
        "/api/v1/session":{"post":{"summary":"Authenticate with resolved administrator token","parameters":[origin.clone()],"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","additionalProperties":false,"required":["token"],"properties":{"token":{"type":"string","writeOnly":true,"minLength":32,"maxLength":256}}}}}},"responses":session_responses.clone()},"get":{"summary":"Bootstrap current session and CSRF after page refresh","security":[{"session":[]}],"responses":session_responses},"delete":{"summary":"Revoke session","security":[{"session":[]}],"parameters":[origin.clone(),csrf.clone()],"responses":logout_responses}},
        "/api/v1/manage":{"post":{"summary":"Forward an allowlisted host management call","security":[{"session":[]}],"parameters":[origin,csrf],"requestBody":{"required":true,"content":{"application/json":{"schema":{"type":"object","additionalProperties":false,"required":["method","payload"],"properties":{"method":{"type":"string","maxLength":128},"payload":{"type":"object"}}}}}},"responses":manage_responses}}
    },"components":{"securitySchemes":{"session":{"type":"apiKey","in":"cookie","name":"twintail_session"}},"schemas":{"Session":{"type":"object","required":["csrf_token","expires_in_secs"],"properties":{"csrf_token":{"type":"string","pattern":"^[0-9a-f]{64}$"},"expires_in_secs":{"type":"integer","minimum":0}}},"Error":{"type":"object","required":["error"],"properties":{"error":{"type":"object","required":["code","message"],"properties":{"code":{"type":"string"},"message":{"type":"string"}}}}}}}});
    let methods = json!([
        "management.overview",
        "plugins.list",
        "plugins.status",
        "plugins.installed",
        "plugins.inspect",
        "operations.list",
        "operations.get",
        "config.get",
        "config.schema",
        "config.validate",
        "diagnostics.logs",
        "adapter.status",
        "plugins.install",
        "plugins.enable",
        "plugins.disable",
        "plugins.uninstall",
        "plugins.reload",
        "plugins.start",
        "plugins.stop",
        "config.update"
    ]);
    api["paths"]["/api/v1/manage"]["post"]["requestBody"]["content"]["application/json"]["schema"]
        ["properties"]["method"]["enum"] = methods;
    api["paths"]["/api/v1/manage"]["post"]["description"] = json!(
        "Read capability: management.overview, plugins.list/status/installed/inspect, operations.list/get, config.get/schema/validate, diagnostics.logs, adapter.status. Write capability: plugins.install/enable/disable/uninstall/reload/start/stop and config.update. Read-only grants receive 403 on writes. Targeted calls use payload.id; operations.get uses operation id. Config input uses unresolved values, never resolved secrets. Lifecycle/config mutations immediately return result OperationAck: poll operations.get until completed/failed/interrupted. A 504 or lost response does not prove mutation failure; use operations.list and the same request_key rather than blindly retrying. Results and payloads vary by method."
    );
    api["paths"]["/api/v1/manage"]["post"]["requestBody"]["content"]["application/json"]["schema"]
        ["properties"]["payload"] = json!({"type":"object","description":"Method-specific payload: id selects plugin/operation. Install uses path; enable uses id, optional version and grants. Mutations accept optional request_key (1..128 ASCII alphanumeric/dot/underscore/hyphen) for idempotency; same key with changed payload or actor conflicts. config.validate/update use values; update additionally requires expected_revision. plugin reload may select candidate version. See ConfigMutation and Operation schemas.","properties":{"id":{"type":"string"},"request_key":{"type":"string","pattern":"^[a-zA-Z0-9._-]{1,128}$"},"expected_revision":{"type":"integer","minimum":0},"values":{"type":"object","description":"Full unresolved configuration; retain {$secret: ENV_NAME} environment references. Browser rejects literal writeOnly credentials; migrate through local CLI."}}});
    api["components"]["schemas"]["OperationAck"] = json!({"type":"object","required":["id","state"],"properties":{"id":{"type":"string"},"state":{"enum":["running","completed","failed","interrupted"]}},"description":"Acknowledgment of admission, not success. Poll operations.get with id. Same-key retry returns original operation; conflict requires refresh."});
    api["components"]["schemas"]["Operation"] = json!({"type":"object","required":["id","state","stage","method"],"properties":{"id":{"type":"string"},"state":{"enum":["running","completed","failed","interrupted"]},"stage":{"type":"string"},"method":{"type":"string"},"target":{"type":["string","null"]},"actor":{"type":"string"},"request_key":{"type":["string","null"]},"expected_revision":{"type":["integer","null"]},"created_unix_ms":{"type":"integer"},"result":{"type":["object","null"],"description":"Safe outcome projection; status, outcome, revision, needs_recovery and error code as applicable."}}});
    api["components"]["schemas"]["ConfigMutation"] = json!({"type":"object","required":["id","values","expected_revision"],"properties":{"id":{"type":"string"},"values":{"type":"object"},"expected_revision":{"type":"integer","minimum":0},"request_key":{"type":"string"}},"description":"Revision CAS; stale revision returns conflict, possibly in terminal operation result after ACK. Refresh config.get and preserve user edits for explicit revalidation; never auto-overwrite. Restart-required returns a failed operation and preserves safe status for reload."});
    api["components"]["schemas"]["Conflict"] = json!({"type":"object","description":"409: stale revision, busy controller, or reused request_key with changed actor/payload. HTTP error contains safe code/message only; refresh state and inspect operation status.","properties":{"error":{"type":"object","properties":{"code":{"const":"conflict"},"message":{"type":"string"}}}}});
    api["paths"]["/api/v1/manage"]["post"]["responses"]["200"]["content"]["application/json"]["schema"]
        ["properties"]["result"] = json!({"anyOf":[{"$ref":"#/components/schemas/OperationAck"},{"type":["object","array"],"description":"Read result, e.g. operation record, plugin snapshot/list, unresolved configuration/schema, diagnostic projection; distinguished by requested method. Read objects may also contain id/state, use requested method to interpret."}]});
    api
}
