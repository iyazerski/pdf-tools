use tracing::info;

pub(crate) async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{signal, SignalKind};

        let mut term =
            signal(SignalKind::terminate()).expect("register SIGTERM handler must succeed");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {
                info!("shutdown signal received (SIGINT)");
            }
            _ = term.recv() => {
                info!("shutdown signal received (SIGTERM)");
            }
        }
    }

    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
        info!("shutdown signal received");
    }
}
