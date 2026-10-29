//! Test-only fault-injection executable for the P1 supervisor integration suite.
//!
//! This is neither a production plugin nor a plugin SDK. It deliberately speaks
//! JSON directly, independently of the protocol helpers, to exercise the public
//! wire contract. stdout contains only frames; diagnostics use stderr.

use serde_json::{Value, json};
use std::{
    io::{self, Read, Write},
    net::TcpListener,
    path::PathBuf,
    time::Duration,
};

const MAX_FRAME: usize = 8 * 1024 * 1024;

fn send(out: &mut impl Write, message: Value) -> io::Result<()> {
    let bytes = serde_json::to_vec(&message)?;
    out.write_all(&(bytes.len() as u32).to_be_bytes())?;
    out.write_all(&bytes)?;
    out.flush()
}

fn receive(input: &mut impl Read) -> io::Result<Value> {
    let mut prefix = [0; 4];
    input.read_exact(&mut prefix)?;
    let length = u32::from_be_bytes(prefix) as usize;
    if length == 0 || length > MAX_FRAME {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid length"));
    }
    let mut bytes = vec![0; length];
    input.read_exact(&mut bytes)?;
    Ok(serde_json::from_slice(&bytes)?)
}

fn envelope(kind: &str, method: &str, payload: Value) -> Value {
    json!({"protocol_version":{"major":1,"minor":0},"kind":kind,
        "method":method,"payload":payload})
}

fn expect_control(input: &mut impl Read, method: &str) -> io::Result<Value> {
    let message = receive(input)?;
    if message["kind"] != "control" || message["method"] != method {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "unexpected control",
        ));
    }
    Ok(message["payload"].clone())
}

fn run() -> io::Result<()> {
    let mut args = std::env::args().skip(1);
    let mode = args.next().unwrap_or_else(|| "normal".into());
    let id = args.next().unwrap_or_else(|| "fixture".into());
    let delay = args.next();
    let marker = args.next().map(PathBuf::from);
    let mut input = io::stdin().lock();
    let mut output = io::stdout();
    match mode.as_str() {
        "handshake-timeout" => {
            // Wait for the host to close stdin after its handshake deadline.
            let _ = receive(&mut input);
            return Ok(());
        }
        "oversize" => {
            output.write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())?;
            output.flush()?;
            let _ = receive(&mut input);
            return Ok(());
        }
        "bad-json" => {
            output.write_all(&1u32.to_be_bytes())?;
            output.write_all(b"{")?;
            output.flush()?;
            let _ = receive(&mut input);
            return Ok(());
        }
        _ => {}
    }
    let mut hello = envelope(
        "control",
        "hello",
        json!({
            "plugin_id":id,"version":if mode.ends_with("-v2") { "0.2.0" } else { "0.1.0" },"required_capabilities":[]
        }),
    );
    if mode == "management" {
        hello["protocol_version"]["minor"] = json!(2);
    }
    if mode == "wrong-version" {
        hello["protocol_version"]["major"] = json!(2);
    }
    send(&mut output, hello)?;
    let negotiated = expect_control(&mut input, "negotiate")?;
    if negotiated != json!({"major":1,"minor":if mode == "management" { 2 } else { 0 }}) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "P1 minor negotiation mismatch",
        ));
    }
    let initialize = expect_control(&mut input, "initialize")?;
    let data = PathBuf::from(
        initialize["data_dir"]
            .as_str()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing data directory"))?,
    );
    if mode == "start-once" {
        let seen = data.join("fixture-start-once");
        if seen.exists() {
            return Err(io::Error::other("fixture cannot recover"));
        }
        std::fs::write(seen, b"started")?;
    }
    // Holding the same loopback port across a generation gives tests an OS-owned
    // resource whose release is observable after a forced kill, without unsafe.
    let _lease = if matches!(mode.as_str(), "ignore-shutdown" | "ignore-shutdown-v2") {
        let address_file = data.join("fixture-address");
        let listener = if address_file.is_file() {
            match TcpListener::bind(std::fs::read_to_string(&address_file)?) {
                Ok(listener) => listener,
                Err(error) => {
                    std::fs::write(
                        data.join("fixture-overlap"),
                        b"old instance still owns port",
                    )?;
                    return Err(error);
                }
            }
        } else {
            TcpListener::bind("127.0.0.1:0")?
        };
        std::fs::write(&address_file, listener.local_addr()?.to_string())?;
        Some(listener)
    } else {
        None
    };
    if mode == "held-handshake" {
        let marker = marker
            .as_ref()
            .ok_or_else(|| io::Error::other("missing handshake barrier"))?;
        std::fs::write(marker, b"handshake held")?;
        while !marker.with_extension("release").is_file() {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    if mode == "slow-handshake" {
        if let Some(marker) = &marker {
            std::fs::write(marker, b"handshake pending")?;
        }
        std::thread::sleep(Duration::from_millis(
            delay
                .as_deref()
                .unwrap_or("200")
                .parse()
                .map_err(io::Error::other)?,
        ));
        // Snapshot an externally observable effect before sending ready. This
        // distinguishes periodic reconciliation from start's final reconciliation.
        if let Some(marker) = &marker {
            std::fs::write(
                data.join("fixture-dependent-stopped-before-ready"),
                if marker.with_extension("stopped").is_file() {
                    "yes"
                } else {
                    "no"
                },
            )?;
        }
    }
    send(&mut output, envelope("control", "ready", json!({})))?;
    expect_control(&mut input, "activate")?;
    if mode == "management" {
        let trigger = marker
            .clone()
            .ok_or_else(|| io::Error::other("missing management trigger"))?;
        let payload: Value = serde_json::from_str(
            delay
                .as_deref()
                .ok_or_else(|| io::Error::other("missing management payload"))?,
        )?;
        std::thread::spawn(move || {
            while !trigger.is_file() {
                std::thread::sleep(Duration::from_millis(2));
            }
            let mut message = envelope("request", "management.call", payload);
            message["request_id"] = json!("fixture-management");
            let _ = send(&mut io::stdout(), message);
        });
    }
    if mode == "crash-on-file" {
        let trigger = marker
            .clone()
            .ok_or_else(|| io::Error::other("missing crash trigger"))?;
        std::thread::spawn(move || {
            while !trigger.is_file() {
                std::thread::sleep(Duration::from_millis(2));
            }
            std::process::exit(23);
        });
    }
    if matches!(mode.as_str(), "flood" | "control-flood") {
        // Do not drain stdin: the host must bound responses/control even when
        // the peer continues sending but stops consuming its replies.
        for sequence in 0..2048 {
            let mut message = if mode == "flood" {
                envelope("request", "host.health", json!({}))
            } else {
                envelope("control", "ping", json!({}))
            };
            if mode == "flood" {
                message["request_id"] = json!(format!("flood-{sequence}"));
            }
            send(&mut output, message)?;
        }
        loop {
            std::thread::park();
        }
    }
    if mode == "crash" {
        std::process::exit(23);
    }
    if matches!(mode.as_str(), "permission" | "granted" | "duplicate-id") {
        let mut request = envelope("request", "host.health", json!({}));
        request["request_id"] = json!("fixture-health");
        send(&mut output, request.clone())?;
        if mode == "duplicate-id" {
            send(&mut output, request)?;
        }
    }
    if mode == "unknown-response" {
        send(
            &mut output,
            json!({"protocol_version":{"major":1,"minor":0},
            "kind":"response","request_id":"never-issued",
            "outcome":{"status":"ok","value":{}}}),
        )?;
    }
    let mut health = Value::Null;
    let mut waiting_probes = Vec::new();
    loop {
        let message = receive(&mut input)?;
        match message["kind"].as_str() {
            Some("control") if message["method"] == "shutdown" => {
                std::fs::write(data.join("fixture-shutdown-received"), b"shutdown")?;
                if matches!(mode.as_str(), "ignore-shutdown" | "ignore-shutdown-v2") {
                    continue;
                }
                std::fs::write(data.join("fixture-stopped"), b"shutdown")?;
                if let Some(marker) = &marker {
                    std::fs::write(marker.with_extension("stopped"), b"shutdown")?;
                }
                return Ok(());
            }
            Some("response") => {
                if mode == "management" && message["request_id"] == "fixture-management" {
                    std::fs::write(
                        data.join("fixture-management-result.json"),
                        serde_json::to_vec(&message["outcome"])?,
                    )?;
                }
                health = message["outcome"].clone();
                for request_id in waiting_probes.drain(..) {
                    respond(&mut output, request_id, &initialize, &health)?;
                }
            }
            Some("request")
                if mode == "management"
                    && matches!(
                        message["method"].as_str(),
                        Some("lifecycle.started" | "config.status")
                    ) =>
            {
                let value = if message["method"] == "config.status" {
                    json!({"applied_revision":initialize["config"]["revision"],"config_version":initialize["config"]["config_version"]})
                } else {
                    json!({"started":true,"config_revision":initialize["config"]["revision"]})
                };
                send(
                    &mut output,
                    json!({"protocol_version":{"major":1,"minor":2},"kind":"response","request_id":message["request_id"],"outcome":{"status":"ok","value":value}}),
                )?;
            }
            Some("request") if message["method"] == "probe" => {
                if mode == "response-timeout" {
                    continue;
                }
                if mode == "late" && initialize["generation"] == 1 {
                    std::thread::sleep(Duration::from_millis(
                        delay
                            .as_deref()
                            .unwrap_or("175")
                            .parse()
                            .map_err(io::Error::other)?,
                    ));
                    respond(
                        &mut output,
                        message["request_id"].clone(),
                        &initialize,
                        &health,
                    )?;
                    std::fs::write(data.join("fixture-late-response"), b"generation 1")?;
                    continue;
                }
                let request_id = message["request_id"].clone();
                if matches!(mode.as_str(), "permission" | "granted") && health.is_null() {
                    // The host's control queue can deliver probe before health's response.
                    waiting_probes.push(request_id);
                } else {
                    respond(&mut output, request_id, &initialize, &health)?;
                }
            }
            _ => {}
        }
    }
}

fn respond(
    out: &mut impl Write,
    request_id: Value,
    init: &Value,
    health: &Value,
) -> io::Result<()> {
    send(
        out,
        json!({"protocol_version":{"major":1,"minor":0},"kind":"response",
        "request_id":request_id,"outcome":{"status":"ok","value":{
            "generation":init["generation"],"instance_id":init["instance_id"],
            "grants":init["granted_capabilities"],"health":health}}}),
    )
}

fn main() {
    if let Err(error) = run() {
        eprintln!("p1 test fixture: {error}");
        // Keep diagnostics bounded and let the host own process cleanup.
        std::thread::sleep(Duration::from_millis(5));
        std::process::exit(1);
    }
}
