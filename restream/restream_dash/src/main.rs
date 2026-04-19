use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::time::Duration;

use axum::Router;
use axum::routing::{get, post};
use tokio::sync::{RwLock, broadcast};
use tower_http::services::ServeDir;
use tracing::{info, warn};

mod config;
mod handlers;
mod services;
mod state;
mod utils;
mod workers;

use config::EnvConfig;
use handlers::admin::{admin_page, update_source};
use handlers::debug::debug_mpd;
use handlers::health::health;
use handlers::proxy::{proxy_manifest, proxy_segment};
use handlers::stream::stream_playlist;
use services::hls_cleanup::prepare_hls_dir;
use services::url_refresher::run_url_refresh_loop;
use state::{AppState, SourceConfig};
use utils::shutdown::shutdown_signal;
use workers::ffmpeg::run_ffmpeg_supervisor;

#[tokio::main]
async fn main() {
    let env = EnvConfig::from_env();

    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG")
                .unwrap_or_else(|_| "restream_dash=info,axum=info,tower_http=info".to_string()),
        )
        .init();

    if env.jwt_token.is_empty() {
        warn!("JWT_TOKEN not set; automatic URL refresh disabled");
    }

    if env.decryption_key.is_none() {
        warn!("no decryption key found; CENC segments will not be decrypted");
    } else {
        info!("decryption key loaded");
    }

    prepare_hls_dir(&env.hls_dir).await;

    let (restart_tx, _) = broadcast::channel::<()>(8);

    let http_client = reqwest::Client::builder()
        .pool_max_idle_per_host(8)
        .tcp_keepalive(Duration::from_secs(60))
        .timeout(Duration::from_secs(30))
        .build()
        .expect("failed to build reqwest client");

    let app_state = AppState {
        source_config: Arc::new(RwLock::new(SourceConfig {
            source_url: env.source_url,
            source_origin: env.source_origin,
            source_referer: env.source_referer,
        })),
        source_user_agent: env.source_user_agent,
        source_accept: env.source_accept,
        source_accept_language: env.source_accept_language,
        source_sec_ch_ua: env.source_sec_ch_ua,
        source_sec_ch_ua_mobile: env.source_sec_ch_ua_mobile,
        source_sec_ch_ua_platform: env.source_sec_ch_ua_platform,
        source_sec_fetch_dest: env.source_sec_fetch_dest,
        source_sec_fetch_mode: env.source_sec_fetch_mode,
        source_sec_fetch_site: env.source_sec_fetch_site,
        decryption_key: Arc::new(RwLock::new(env.decryption_key)),
        jwt_token: Arc::new(RwLock::new(env.jwt_token)),
        tbxapis_content_id: env.tbxapis_content_id,
        restart_tx,
        http_client,
        max_video_seq: Arc::new(AtomicU64::new(0)),
        max_audio_seq: Arc::new(AtomicU64::new(0)),
    };

    if !app_state.jwt_token.read().await.is_empty() {
        tokio::spawn(run_url_refresh_loop(app_state.clone()));
    }

    tokio::spawn(run_ffmpeg_supervisor(
        app_state.clone(),
        env.hls_dir.clone(),
    ));

    let app = Router::new()
        .route("/health", get(health))
        .route("/admin", get(admin_page))
        .route("/admin/source", post(update_source))
        .route("/stream.m3u8", get(stream_playlist))
        .route("/debug/mpd", get(debug_mpd))
        .route("/dash-proxy-manifest", get(proxy_manifest))
        .route("/dash-proxy/{*path}", get(proxy_segment))
        .nest_service("/hls", ServeDir::new(env.hls_dir))
        .with_state(app_state);

    let addr: SocketAddr = env.bind_addr.parse().expect("invalid bind address");
    info!(%addr, "listening");

    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .expect("failed to bind listener");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("server error");
}
