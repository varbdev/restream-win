use std::path::PathBuf;
use std::process::Stdio;
use std::sync::atomic::Ordering;
use std::time::Duration;

use tokio::process::Command;
use tracing::{error, info, warn};

use crate::handlers::proxy::clear_init_cache;
use crate::services::hls_cleanup::cleanup_hls_files;
use crate::state::AppState;

const PROXY_MANIFEST_URL: &str = "http://127.0.0.1:3001/dash-proxy-manifest";

pub async fn run_ffmpeg_supervisor(state: AppState, hls_dir: PathBuf) {
    let mut restart_delay = Duration::from_secs(1);
    let max_restart_delay = Duration::from_secs(30);
    let mut restart_rx = state.restart_tx.subscribe();
    let mut is_first_start = true;

    loop {
        let source_url = state.source_config.read().await.source_url.clone();

        if source_url.is_empty() {
            warn!("source URL is empty; waiting for URL refresh...");
            tokio::time::sleep(Duration::from_secs(5)).await;
            continue;
        }

        let started_at = tokio::time::Instant::now();
        let playlist_path = hls_dir.join("stream.m3u8");
        let segment_filename = hls_dir.join("stream_%06d.ts");

        if is_first_start {
            cleanup_hls_files(&hls_dir).await;
            clear_init_cache();
            is_first_start = false;
        }

        let args = build_ffmpeg_args(
            playlist_path.to_str().expect("invalid hls path"),
            segment_filename.to_str().expect("invalid hls segment path"),
        );
        info!(command = ?args, "ffmpeg command");

        let mut cmd = Command::new("ffmpeg");
        cmd.kill_on_drop(true);
        cmd.args(&args);
        cmd.stdout(Stdio::null());
        cmd.stderr(Stdio::inherit());
        cmd.stdin(Stdio::null());

        let mut child = match cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                error!(error = %err, "failed to spawn ffmpeg");
                tokio::time::sleep(Duration::from_secs(2)).await;
                continue;
            }
        };

        let mut forced_restart = false;

        tokio::select! {
            result = child.wait() => {
                match result {
                    Ok(status) => warn!(
                        %status,
                        ran_for_secs = started_at.elapsed().as_secs(),
                        delay_secs = restart_delay.as_secs(),
                        "ffmpeg exited; restarting"
                    ),
                    Err(err) => error!(
                        error = %err,
                        ran_for_secs = started_at.elapsed().as_secs(),
                        "ffmpeg wait failed; restarting"
                    ),
                }
            }
            result = restart_rx.recv() => {
                match result {
                    Ok(()) => {
                        info!("ffmpeg restart requested");
                        forced_restart = true;
                        if let Err(err) = child.start_kill() {
                            warn!(error = %err, "failed to kill ffmpeg");
                        }
                        let _ = child.wait().await;
                        cleanup_hls_files(&hls_dir).await;
                        clear_init_cache();
                        state.max_video_seq.store(0, Ordering::Relaxed);
                        state.max_audio_seq.store(0, Ordering::Relaxed);
                    }
                    Err(err) => warn!(error = %err, "restart channel closed"),
                }
            }
        }

        if forced_restart {
            restart_delay = Duration::from_secs(1);
            continue;
        }

        tokio::time::sleep(restart_delay).await;
        if started_at.elapsed() >= Duration::from_secs(20) {
            restart_delay = Duration::from_secs(1);
        } else {
            restart_delay = std::cmp::min(restart_delay.saturating_mul(2), max_restart_delay);
        }
    }
}

const AUDIO_FILTER: &str = "aresample=async=1:min_hard_comp=0.100:first_pts=0,asetpts=PTS-STARTPTS";

fn build_ffmpeg_args(playlist_path: &str, segment_filename: &str) -> Vec<String> {
    vec![
        "-protocol_whitelist".into(),
        "file,crypto,data,https,http,tcp,tls".into(),
        "-rw_timeout".into(),
        "15000000".into(),
        "-fflags".into(),
        "+genpts+igndts+discardcorrupt".into(),
        "-err_detect".into(),
        "ignore_err".into(),
        "-thread_queue_size".into(),
        "4096".into(),
        "-i".into(),
        PROXY_MANIFEST_URL.into(),
        "-map".into(),
        "0:v:0?".into(),
        "-map".into(),
        "0:a:0?".into(),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        "veryfast".into(),
        "-tune".into(),
        "zerolatency".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-profile:v".into(),
        "main".into(),
        "-g".into(),
        "48".into(),
        "-keyint_min".into(),
        "48".into(),
        "-sc_threshold".into(),
        "0".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "128k".into(),
        "-ac".into(),
        "2".into(),
        "-ar".into(),
        "48000".into(),
        "-af".into(),
        AUDIO_FILTER.into(),
        "-max_interleave_delta".into(),
        "0".into(),
        "-max_muxing_queue_size".into(),
        "4096".into(),
        "-f".into(),
        "hls".into(),
        "-hls_time".into(),
        "4".into(),
        "-hls_list_size".into(),
        "10".into(),
        "-hls_segment_filename".into(),
        segment_filename.into(),
        "-hls_flags".into(),
        "delete_segments+temp_file+independent_segments".into(),
        "-hls_allow_cache".into(),
        "0".into(),
        "-y".into(),
        playlist_path.into(),
    ]
}
