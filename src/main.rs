//! Nyquest v3.1 — Semantic Compression Proxy for LLMs
//! Full Rust Stack with integrated CLI installer

use clap::Parser;
use std::sync::Arc;
use tracing::info;

use nyquest::cli::{Cli, Commands, ConfigAction};

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    let config_path = cli.config.clone();

    match cli.command {
        // ── CLI Commands (no server, no tracing) ──
        Some(Commands::Install {
            defaults,
            overrides,
        }) => {
            nyquest::cli::install::run_install(&config_path, defaults, &overrides);
        }
        Some(Commands::Configure { section }) => {
            nyquest::cli::install::run_configure(&config_path, section.as_deref());
        }
        Some(Commands::Doctor) => {
            nyquest::cli::doctor::run_doctor(&config_path);
        }
        Some(Commands::Status) => {
            nyquest::cli::doctor::run_status(&config_path);
        }
        Some(Commands::Preflight { verbose }) => {
            nyquest::cli::preflight::run_preflight(&config_path, verbose);
        }
        Some(Commands::Config { action }) => match action {
            ConfigAction::Show => nyquest::cli::config_cmd::run_show(&config_path),
            ConfigAction::Get { key } => nyquest::cli::config_cmd::run_get(&config_path, &key),
            ConfigAction::Set { key, value } => {
                nyquest::cli::config_cmd::run_set(&config_path, &key, &value)
            }
        },

        // ── Server Mode (default or explicit `serve`) ──
        Some(Commands::Serve) | None => {
            run_server(&config_path).await;
        }
    }
}

async fn run_server(config_path: &str) {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "nyquest=info".into()),
        )
        .init();

    let config = nyquest::config::load_config(Some(config_path));

    info!(
        "Nyquest v{} starting on {}:{}",
        nyquest::VERSION,
        config.host,
        config.port
    );
    info!("Target API: {}", config.target_api_base);
    info!("Default compression level: {}", config.compression_level);
    info!(
        "Normalization: {} | Boundaries: {}",
        config.normalize, config.inject_boundaries
    );

    let http_client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(config.request_timeout))
        .pool_max_idle_per_host(10)
        .build()
        .expect("Failed to build HTTP client");

    // Initialize semantic compression engine
    let sem_config = config.semantic_config();
    let mut semantic_engine = nyquest::semantic::SemanticEngine::new(sem_config);
    if config.semantic_enabled {
        let ok = semantic_engine.health_check().await;
        if ok {
            info!("Semantic engine: ONLINE (model: {})", config.semantic_model);
        } else {
            info!(
                "Semantic engine: OFFLINE — falling back to {}",
                config.semantic_fallback
            );
        }
    } else {
        info!("Semantic engine: disabled");
    }

    let state = Arc::new(nyquest::server::AppState {
        token_counter: nyquest::tokens::TokenCounter::new(),
        metrics_logger: nyquest::tokens::MetricsLogger::new(&config.log_file),
        http_client,
        analytics: nyquest::analytics::RuleAnalytics::new(),
        semantic: tokio::sync::Mutex::new(semantic_engine),
        config: config.clone(),
    });

    let app = nyquest::server::build_router(state);

    let addr = format!("{}:{}", config.host, config.port);
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .unwrap_or_else(|_| panic!("Failed to bind to {}", addr));

    info!("Nyquest v{} listening on {}", nyquest::VERSION, addr);

    axum::serve(listener, app).await.expect("Server error");
}
