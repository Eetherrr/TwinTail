//! Bounded TCP task group. Shutdown drops live connections and joins every task.
use crate::{Config, Management, http::router};
use hyper_util::{
    rt::{TokioExecutor, TokioIo},
    server::conn::auto::Builder,
    service::TowerToHyperService,
};
use std::{net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    net::TcpListener,
    sync::{Mutex, Semaphore, watch},
    task::{JoinHandle, JoinSet},
};
use twintail_plugin_sdk::{ErrorCode, Result, RpcError};

/// Owns the listener, live connections and cancellation-safe join handle.
pub struct Server {
    address: SocketAddr,
    cancel: watch::Sender<bool>,
    join: Mutex<Option<JoinHandle<()>>>,
}
impl Server {
    /// Binds before returning, so SDK start reports port conflicts to the host.
    pub async fn start(config: Config, backend: Arc<dyn Management>) -> Result<Self> {
        config.validate()?;
        let listener = TcpListener::bind(config.bind).await.map_err(|_| {
            RpcError::new(
                ErrorCode::InvalidConfig,
                "web administrator listener unavailable",
            )
        })?;
        let address = listener
            .local_addr()
            .map_err(|_| RpcError::new(ErrorCode::RemoteError, "HTTP address unavailable"))?;
        let app = router(config.clone(), address, backend)?;
        let (cancel, mut cancelled) = watch::channel(false);
        let limit = config.max_connections;
        let join = tokio::spawn(async move {
            let permits = Arc::new(Semaphore::new(limit));
            let mut connections = JoinSet::new();
            loop {
                tokio::select! {biased;
                    _=wait_cancel(&mut cancelled)=>break,
                    Some(_)=connections.join_next(),if !connections.is_empty()=>{},
                    permit=permits.clone().acquire_owned()=>{
                        let Ok(permit)=permit else{break;};
                        let accepted=tokio::select!{biased;_=wait_cancel(&mut cancelled)=>break,accepted=listener.accept()=>accepted};
                        let Ok((socket,peer))=accepted else{tokio::time::sleep(Duration::from_millis(10)).await;continue;};
                        if !peer.ip().is_loopback(){continue;}
                        let service=TowerToHyperService::new(app.clone());let mut stop=cancelled.clone();
                        connections.spawn(async move {
                            let _permit=permit;let mut builder=Builder::new(TokioExecutor::new());
                            builder.http1().header_read_timeout(Duration::from_secs(5)).timer(hyper_util::rt::TokioTimer::new()).max_buf_size(16384);
                            let connection=builder.serve_connection(TokioIo::new(socket),service);
                            tokio::select!{biased;_=wait_cancel(&mut stop)=>{},_=connection=>{}}
                        });
                    }
                }
            }
            // Drop listener before join to release the exclusive port immediately.
            drop(listener);
            connections.abort_all();
            while connections.join_next().await.is_some() {}
        });
        Ok(Self {
            address,
            cancel,
            join: Mutex::new(Some(join)),
        })
    }
    /// Actual bound address. Do not expose other configuration fields to diagnostics.
    pub fn address(&self) -> SocketAddr {
        self.address
    }
    /// Cancels and joins all TCP tasks; cancellation of this future retains ownership.
    pub async fn stop(&self) {
        self.cancel.send_replace(true);
        let mut join = self.join.lock().await;
        if let Some(task) = join.as_mut() {
            let _ = task.await;
        }
        join.take();
    }
}
async fn wait_cancel(cancel: &mut watch::Receiver<bool>) {
    if !*cancel.borrow_and_update() {
        let _ = cancel.changed().await;
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.cancel.send_replace(true);
    }
}
