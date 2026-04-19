use bytes::Bytes;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::fs;
use tokio::process::Command;
use tracing::error;

static DECRYPT_COUNTER: AtomicU64 = AtomicU64::new(0);

pub fn extract_base_url(mpd_url: &str) -> String {
    match mpd_url.rfind('/') {
        Some(idx) => mpd_url[..=idx].to_string(),
        None => mpd_url.to_string(),
    }
}

pub fn patch_mpd(mpd_xml: &str, proxy_base_url: &str) -> String {
    let mut result = String::with_capacity(mpd_xml.len() + 128);
    let mut inside_content_protection = false;
    let mut inside_base_url = false;

    for line in mpd_xml.lines() {
        let trimmed = line.trim();

        if inside_content_protection {
            if trimmed.contains("</ContentProtection>") {
                inside_content_protection = false;
            }
            continue;
        }

        if trimmed.starts_with("<ContentProtection") {
            if !trimmed.ends_with("/>") && !trimmed.contains("</ContentProtection>") {
                inside_content_protection = true;
            }
            continue;
        }

        if inside_base_url {
            if trimmed.contains("</BaseURL>") {
                inside_base_url = false;
            }
            continue;
        }

        if trimmed.starts_with("<BaseURL") {
            if trimmed.contains("</BaseURL>") || trimmed.ends_with("/>") {
                continue;
            }
            inside_base_url = true;
            continue;
        }

        result.push_str(line);
        result.push('\n');

        if trimmed.starts_with("<Period ") || trimmed == "<Period>" {
            result.push_str(&format!("    <BaseURL>{}</BaseURL>\n", proxy_base_url));
        }
    }

    result
}

pub async fn fetch_segment(
    client: &reqwest::Client,
    url: &str,
    user_agent: &str,
    origin: &str,
    referer: &str,
) -> Result<Bytes, String> {
    let response = client
        .get(url)
        .header("accept", "*/*")
        .header("origin", origin)
        .header("referer", referer)
        .header("user-agent", user_agent)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| e.to_string())?;

    if !response.status().is_success() {
        return Err(format!("CDN returned {}", response.status()));
    }

    response.bytes().await.map_err(|e| e.to_string())
}

pub async fn strip_cenc_from_init(data: Bytes, kid: &str, key: &str) -> Result<Bytes, String> {
    let id = DECRYPT_COUNTER.fetch_add(1, Ordering::Relaxed);

    let tmp_in = format!("/tmp/init_enc_{}.mp4", id);
    let tmp_out = format!("/tmp/init_dec_{}.mp4", id);

    fs::write(&tmp_in, &data).await.map_err(|e| e.to_string())?;

    let status = Command::new("mp4decrypt")
        .args(["--key", &format!("{}:{}", kid, key), &tmp_in, &tmp_out])
        .status()
        .await
        .map_err(|e| {
            error!(error = %e, "failed to spawn mp4decrypt for init");
            e.to_string()
        })?;

    let _ = fs::remove_file(&tmp_in).await;

    if !status.success() {
        let _ = fs::remove_file(&tmp_out).await;
        return Err(format!("mp4decrypt init exited with {}", status));
    }

    let stripped = fs::read(&tmp_out).await.map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&tmp_out).await;

    Ok(Bytes::from(stripped))
}

pub async fn decrypt_mp4(
    data: Bytes,
    init_data: &Bytes,
    kid: &str,
    key: &str,
) -> Result<Bytes, String> {
    let id = DECRYPT_COUNTER.fetch_add(1, Ordering::Relaxed);

    let tmp_init = format!("/tmp/fraginit_{}.mp4", id);
    let tmp_in = format!("/tmp/enc_{}.mp4", id);
    let tmp_out = format!("/tmp/dec_{}.mp4", id);

    fs::write(&tmp_init, init_data.as_ref())
        .await
        .map_err(|e| e.to_string())?;
    fs::write(&tmp_in, &data).await.map_err(|e| e.to_string())?;

    let status = Command::new("mp4decrypt")
        .args([
            "--fragments-info",
            &tmp_init,
            "--key",
            &format!("{}:{}", kid, key),
            &tmp_in,
            &tmp_out,
        ])
        .status()
        .await
        .map_err(|e| {
            error!(error = %e, "failed to spawn mp4decrypt");
            e.to_string()
        })?;

    let _ = fs::remove_file(&tmp_init).await;
    let _ = fs::remove_file(&tmp_in).await;

    if !status.success() {
        let _ = fs::remove_file(&tmp_out).await;
        return Err(format!("mp4decrypt exited with {}", status));
    }

    let decrypted = fs::read(&tmp_out).await.map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&tmp_out).await;

    Ok(Bytes::from(decrypted))
}
