use std::path::PathBuf;

use tracing::{error, info, warn};

pub async fn prepare_hls_dir(hls_dir: &PathBuf) {
    if let Err(err) = tokio::fs::create_dir_all(hls_dir).await {
        error!(error = %err, path = %hls_dir.display(), "failed to create hls dir");
        return;
    }

    cleanup_hls_files(hls_dir).await;
}

pub async fn cleanup_hls_files(hls_dir: &PathBuf) {
    let mut removed = 0usize;

    let mut dir = match tokio::fs::read_dir(hls_dir).await {
        Ok(dir) => dir,
        Err(err) => {
            error!(error = %err, path = %hls_dir.display(), "failed to read hls dir");
            return;
        }
    };

    while let Ok(Some(entry)) = dir.next_entry().await {
        let path = entry.path();
        let file_name = entry.file_name();
        let name = file_name.to_string_lossy();

        if !(name.ends_with(".ts")
            || name.ends_with(".m3u8")
            || name.ends_with(".m4s")
            || name.ends_with(".mp4")
            || name.ends_with(".tmp"))
        {
            continue;
        }

        if let Err(err) = tokio::fs::remove_file(&path).await {
            warn!(error = %err, path = %path.display(), "failed to remove stale hls file");
            continue;
        }

        removed += 1;
    }

    if removed > 0 {
        info!(removed, path = %hls_dir.display(), "removed stale hls files before ffmpeg start");
    }
}
