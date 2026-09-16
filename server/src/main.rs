//! `allium-deck-server` — an HTTP front end for the deck recommendation engine.
//!
//! The engine is a pure computation library; this binary adds the parts a network
//! service needs around it: masterdata resident in memory, a bounded pool of search
//! threads, per-request ceilings, and observability.

#![deny(clippy::unwrap_used)]

use std::sync::Arc;
use std::time::Duration;

use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::TraceLayer;

use allium_deck_server::api::{self, AppState};
use allium_deck_server::config::{self, Parsed};
use allium_deck_server::metrics::Metrics;
use allium_deck_server::pool::SearchPool;
use allium_deck_server::state::{Registry, SharedRegistry};

// The allocator is selected at build time. The default is measured, not assumed; see
// docker/README.en.md for the comparison and how to reproduce it on your own traffic.
#[cfg(all(feature = "jemalloc", feature = "mimalloc"))]
compile_error!(
    "features `jemalloc` and `mimalloc` both install a global allocator; enable at most one"
);

#[cfg(all(feature = "jemalloc", not(target_env = "msvc")))]
#[global_allocator]
static GLOBAL: tikv_jemallocator::Jemalloc = tikv_jemallocator::Jemalloc;

#[cfg(feature = "mimalloc")]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Headroom over the search deadline for pool building and response serialization.
const EXECUTION_HEADROOM: Duration = Duration::from_secs(10);

fn main() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let config = match config::parse(&args) {
        Ok(Parsed::Run(config)) => *config,
        Ok(Parsed::Help(text)) | Ok(Parsed::Version(text)) => {
            println!("{text}");
            return std::process::ExitCode::SUCCESS;
        }
        Err(error) => {
            eprintln!("error: {error}\n\n{}", config::HELP);
            return std::process::ExitCode::FAILURE;
        }
    };

    init_logging(config.log_json);

    let runtime = match tokio::runtime::Builder::new_multi_thread()
        // The runtime only moves bytes; the searches run on their own threads, so a
        // small number of async workers is enough and leaves the cores for the pool.
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            tracing::error!("building the async runtime failed: {error}");
            return std::process::ExitCode::FAILURE;
        }
    };

    match runtime.block_on(serve(config)) {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!("{error}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn serve(config: config::Config) -> Result<(), String> {
    let started = std::time::Instant::now();
    let registry = Registry::load(&config.regions, &config.default_region)?;
    for snapshot in registry.snapshots() {
        tracing::info!(
            region = %snapshot.name,
            cards = snapshot.counts.cards,
            events = snapshot.counts.events,
            load_ms = snapshot.load_ms,
            "masterdata loaded"
        );
    }

    let search_pool = SearchPool::new(
        config.workers,
        config.max_queue,
        Duration::from_millis(config.queue_timeout_ms),
        Duration::from_millis(config.max_search_timeout_ms) + EXECUTION_HEADROOM,
    );
    let metrics = Metrics::new(Arc::clone(&search_pool.metrics));

    let bind = config.bind;
    let state = Arc::new(AppState {
        registry: SharedRegistry::new(registry),
        pool: search_pool,
        metrics,
        config,
    });

    let app = api::router(Arc::clone(&state))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http());

    let listener = tokio::net::TcpListener::bind(bind)
        .await
        .map_err(|error| format!("binding {bind} failed: {error}"))?;
    tracing::info!(
        %bind,
        workers = state.pool.workers_configured,
        max_queue = state.pool.max_queue,
        startup_ms = started.elapsed().as_secs_f64() * 1000.0,
        "listening"
    );

    let result = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|error| format!("serving failed: {error}"));

    // Connections are drained at this point; let the search threads finish what they
    // already picked up before the process exits.
    state.pool.shutdown();
    tracing::info!("stopped");
    result
}

/// Resolves on SIGTERM (containers) or Ctrl-C (terminals).
async fn shutdown_signal() {
    let interrupt = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut stream) => {
                stream.recv().await;
            }
            Err(error) => tracing::warn!("cannot listen for SIGTERM: {error}"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = interrupt => tracing::info!("interrupt received, draining"),
        () = terminate => tracing::info!("SIGTERM received, draining"),
    }
}

fn init_logging(json: bool) {
    use tracing_subscriber::EnvFilter;

    let filter = EnvFilter::try_from_env("ALLIUM_DECK_LOG")
        .or_else(|_| EnvFilter::try_new("info"))
        .unwrap_or_default();
    let builder = tracing_subscriber::fmt().with_env_filter(filter);
    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}
