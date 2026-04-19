use std::fs;
use std::path::PathBuf;

pub struct EnvConfig {
    pub source_url: String,
    pub source_origin: String,
    pub source_referer: String,
    pub source_user_agent: String,
    pub source_accept: String,
    pub source_accept_language: String,
    pub source_sec_ch_ua: String,
    pub source_sec_ch_ua_mobile: String,
    pub source_sec_ch_ua_platform: String,
    pub source_sec_fetch_dest: String,
    pub source_sec_fetch_mode: String,
    pub source_sec_fetch_site: String,
    pub hls_dir: PathBuf,
    pub bind_addr: String,
    pub jwt_token: String,
    pub tbxapis_content_id: String,
    pub decryption_key: Option<String>,
}

impl EnvConfig {
    pub fn from_env() -> Self {
        let decryption_key = std::env::var("DECRYPTION_KEY")
            .ok()
            .or_else(load_key_from_file);

        Self {
            source_url: std::env::var("SOURCE_URL").unwrap_or_default(),
            source_origin: "https://winplay.co".to_string(),
            source_referer: "https://winplay.co/".to_string(),
            source_user_agent: "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/147.0.0.0 Safari/537.36".to_string(),
            source_accept: "*/*".to_string(),
            source_accept_language: "es,en-US;q=0.9,en;q=0.8".to_string(),
            source_sec_ch_ua: r#""Google Chrome";v="147", "Not.A/Brand";v="8", "Chromium";v="147""#.to_string(),
            source_sec_ch_ua_mobile: "?0".to_string(),
            source_sec_ch_ua_platform: r#""macOS""#.to_string(),
            source_sec_fetch_dest: "empty".to_string(),
            source_sec_fetch_mode: "cors".to_string(),
            source_sec_fetch_site: "cross-site".to_string(),
            hls_dir: PathBuf::from("./media"),
            bind_addr: "0.0.0.0:3001".to_string(),
            jwt_token: std::env::var("JWT_TOKEN").unwrap_or_else(|_| "eyJhbGciOiJIUzI1NiIsInR5cCI6IkpXVCJ9.eyJpc3MiOiJUb29sYm94IERpZ2l0YWwgU0EiLCJhdWQiOiJ1bml0eS50YnhhcGlzLmNvbSIsImlhdCI6MTc3NjUxMzc1NSwiZXhwIjoxNzc2Njg2NTU1LCJjb3VudHJ5IjoiQ08iLCJkZXZpY2VDb3VudHJ5IjoiQ08iLCJsYW5ndWFnZSI6ImVzIiwiY2xpZW50IjoiNmE1NjFkMDQ4NzI4ZGI3Yzc4NmI1M2IwOTQxZDBkZDkiLCJkZXZpY2UiOiI5N2RhNGFlZmU0OWEzZDMyZGQyZWE3YTI3NmFhMjdhODcxMDk1MGM1Iiwic3Vic2NyaWJlciI6IjY3OGU2MTRkODFjYzNhMDAwODY1ZTM4MyIsImluZGV4IjoiNjc2MWI2YWI1NWFkZWYwMjJlZTk3ZDE2IiwiY3VzdG9tZXIiOiI2OWQ1YTZhYmM3ODhlYWZlYmU0ZWU3MWIiLCJwcm9maWxlIjoiNjlkNWE2Y2JmMWNkMDMxMzAwZTBiZWYxIiwibWF4UmF0aW5nIjozfQ.WjCMtXNQTfUMOFmwXCO5DtpO-biSGw370JNt6rOxW-w".to_string()),
            tbxapis_content_id: std::env::var("TBXAPIS_CONTENT_ID")
                .unwrap_or_else(|_| "692dc3d7ddd30f329e90a4cc".to_string()),
            decryption_key,
        }
    }
}

fn load_key_from_file() -> Option<String> {
    let raw = fs::read_to_string("keys.json").ok()?;
    let val: serde_json::Value = serde_json::from_str(&raw).ok()?;
    val.get("keys")?
        .as_array()?
        .first()?
        .get("key")?
        .as_str()
        .map(str::to_string)
}
