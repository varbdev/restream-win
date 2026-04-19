use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use tokio::sync::{RwLock, broadcast};

#[derive(Clone)]
pub struct AppState {
    pub source_config: Arc<RwLock<SourceConfig>>,
    pub source_user_agent: String,
    pub source_accept: String,
    pub source_accept_language: String,
    pub source_sec_ch_ua: String,
    pub source_sec_ch_ua_mobile: String,
    pub source_sec_ch_ua_platform: String,
    pub source_sec_fetch_dest: String,
    pub source_sec_fetch_mode: String,
    pub source_sec_fetch_site: String,
    pub decryption_key: Arc<RwLock<Option<String>>>,
    pub jwt_token: Arc<RwLock<String>>,
    pub tbxapis_content_id: String,
    pub restart_tx: broadcast::Sender<()>,
    pub http_client: reqwest::Client,
    pub max_video_seq: Arc<AtomicU64>,
    pub max_audio_seq: Arc<AtomicU64>,
}

#[derive(Clone)]
pub struct SourceConfig {
    pub source_url: String,
    pub source_origin: String,
    pub source_referer: String,
}
