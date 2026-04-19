use std::time::Duration;

use tokio::time::sleep;
use tracing::{error, info};

use crate::state::AppState;

const TBXAPIS_BASE: &str = "https://unity.tbxapis.com";
const REFRESH_INTERVAL: Duration = Duration::from_secs(60);

pub async fn run_url_refresh_loop(state: AppState) {
    loop {
        match refresh_once(&state).await {
            Ok(new_url) => {
                let mut cfg = state.source_config.write().await;
                if cfg.source_url != new_url {
                    info!(url = %&new_url[..80.min(new_url.len())], "MPD URL refreshed");
                    cfg.source_url = new_url;
                }
            }
            Err(err) => {
                error!(error = %err, "failed to refresh stream URL from tbxapis");
            }
        }
        sleep(REFRESH_INTERVAL).await;
    }
}

async fn refresh_once(state: &AppState) -> Result<String, String> {
    let jwt = state.jwt_token.read().await.clone();
    if jwt.is_empty() {
        return Err("JWT_TOKEN is empty".to_string());
    }

    let client = &state.http_client;
    let url = format!(
        "{}/v0/contents/{}/url",
        TBXAPIS_BASE, state.tbxapis_content_id
    );

    let response = client
        .get(&url)
        .header("accept", "application/json")
        .header("accept-language", "es,en-US;q=0.9,en;q=0.8")
        .header("authorization", format!("JWT {}", jwt))
        .header("content-type", "application/json")
        .header("origin", "https://winplay.co")
        .header("referer", "https://winplay.co/")
        .header("user-agent", "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36")
        .timeout(Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("tbxapis returned {}", response.status()));
    }

    let body: serde_json::Value = response.json().await.map_err(|e| e.to_string())?;

    extract_dash_url(&body).ok_or_else(|| "no DASH entitlement found in response".to_string())
}

fn extract_dash_url(body: &serde_json::Value) -> Option<String> {
    let entitlements = body.get("entitlements")?.as_array()?;

    for entry in entitlements {
        if entry.get("contentType").and_then(|v| v.as_str()) == Some("application/dash+xml") {
            return entry.get("url")?.as_str().map(str::to_string);
        }
    }
    None
}
