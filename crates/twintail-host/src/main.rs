//! TwinTail P3 composition root. All business and adapter code stays in plugins.
mod cli;
mod controller;
mod management;
use cli::{Command, Options};
use std::{io, path::Path, process::ExitCode};
use tokio::sync::{mpsc, watch};
use twintail_core::{Limits, Supervisor, load_plan};
use twintail_protocol::{Body, Message, Outcome, VERSION};
#[tokio::main(flavor = "current_thread")]
async fn main() -> ExitCode {
    let command = match cli::parse(std::env::args_os().skip(1)) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("twintail-host: {e}\nUse --help for usage.");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Command::Help => {
            println!("{}", cli::HELP);
            Ok(())
        }
        Command::Version => {
            println!("twintail-host {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        Command::Run(options) => run(options).await,
        Command::Manage {
            options,
            method,
            payload,
        } => {
            let request = Message::request("cli:1", method, payload);
            match management::call(&options.data_dir, &request).await {
                Ok(message) if message.protocol_version.major == VERSION.major => {
                    match message.body {
                        Body::Response {
                            request_id,
                            outcome,
                        } if request_id == "cli:1" => {
                            println!("{}", serde_json::to_string_pretty(&outcome).unwrap());
                            match outcome {
                                Outcome::Ok(_) => Ok(()),
                                Outcome::Err(e) => Err(io::Error::other(e)),
                            }
                        }
                        _ => Err(io::Error::other("invalid management response")),
                    }
                }
                Ok(_) => Err(io::Error::other("management version mismatch")),
                Err(e) => Err(e),
            }
        }
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("twintail-host: {e}");
            ExitCode::FAILURE
        }
    }
}
fn plan(data: &Path, install: &Path) -> io::Result<Vec<twintail_core::PluginSpec>> {
    let config = data.join("host.toml");
    match config.try_exists()? {
        true => load_plan(&config, install).map_err(io::Error::other),
        false => Ok(vec![]),
    }
}
async fn run(options: Options) -> io::Result<()> {
    let cwd = std::env::current_dir()?;
    let data = cwd.join(options.data_dir);
    let install = cwd.join(options.install_dir);
    if options.check {
        eprintln!(
            "TwinTail {}: empty host ready (bootstrap; no plugins loaded)",
            env!("CARGO_PKG_VERSION")
        );
        eprintln!(
            "data directory: {}\ninstall directory: {}",
            data.display(),
            install.display()
        );
        eprintln!("startup check passed; no directories created or configuration loaded");
        return Ok(());
    }
    let specs = plan(&data, &install)?;
    if options.validate {
        println!(
            "validated {} plugin packages; no code executed",
            specs.len()
        );
        return Ok(());
    }
    // Bind first: a competing host must never spawn the same packages.
    let server = management::LocalServer::bind(&data).await?;
    let shutdown_signal = shutdown_signal()?;
    tokio::pin!(shutdown_signal);
    let supervisor =
        Supervisor::new(specs, data.clone(), Limits::default()).map_err(io::Error::other)?;
    let controller = controller::Controller::new(supervisor, data, install)?;
    let (tx, calls) = mpsc::channel::<management::ManagementCall>(32);
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut server_task = tokio::spawn(server.serve(tx, shutdown_rx.clone()));
    let mut actor = tokio::spawn(controller.serve(calls, shutdown_rx));
    let mut actor_done = false;
    let result = tokio::select! {
        signal = &mut shutdown_signal => signal,
        result = &mut actor => { actor_done = true; result.map_err(io::Error::other) },
        result = &mut server_task => match result { Ok(result) => result, Err(e) => Err(io::Error::other(e)) },
    };
    let _ = shutdown_tx.send(true);
    if !actor_done {
        let _ = actor.await;
    }
    if !server_task.is_finished() {
        let _ = server_task.await;
    }
    eprintln!("TwinTail stopped");
    result
}
#[cfg(unix)]
fn shutdown_signal() -> io::Result<impl Future<Output = io::Result<()>>> {
    use tokio::signal::unix::{SignalKind, signal};
    let mut interrupt = signal(SignalKind::interrupt())?;
    let mut terminate = signal(SignalKind::terminate())?;
    Ok(async move {
        tokio::select! {_=interrupt.recv()=>Ok(()),_=terminate.recv()=>Ok(())}
    })
}
#[cfg(windows)]
fn shutdown_signal() -> io::Result<impl Future<Output = io::Result<()>>> {
    let mut interrupt = tokio::signal::windows::ctrl_c()?;
    Ok(async move {
        interrupt.recv().await;
        Ok(())
    })
}
#[cfg(not(any(unix, windows)))]
fn shutdown_signal() -> io::Result<impl Future<Output = io::Result<()>>> {
    Ok(tokio::signal::ctrl_c())
}
