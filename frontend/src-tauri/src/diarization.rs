//! Post-recording speaker identification ("who said what"), fully on-device.
//!
//! For every transcript segment of a meeting we cut its audio out of the recording,
//! compute Kaldi-style fbank features, run the WeSpeaker ResNet34 speaker-embedding
//! model (ONNX, Apache-2.0), and cluster the embeddings. Each cluster becomes
//! "Speaker N" in `transcripts.speaker`. Renaming/merging speakers is a plain UPDATE.
//!
//! ponytail: one label per transcript segment. Speaker changes *inside* a segment and
//! overlapping speech are not split; that needs a segmentation model (pyannote-style)
//! and is the upgrade path if per-word attribution matters.

use crate::state::AppState;
use anyhow::{anyhow, Context, Result};
use log::{info, warn};
use ndarray::Array3;
use ort::execution_providers::CPUExecutionProvider;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{AppHandle, Emitter, Manager, Runtime};

// Wespeaker/wespeaker-voxceleb-resnet34 at a pinned commit; SHA-256 is HuggingFace's git-lfs oid.
const MODEL_URL: &str = "https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34/resolve/ff1ac5bca8ef11e90662b879aa923979e0bd277b/voxceleb_resnet34.onnx";
const MODEL_FILE: &str = "voxceleb_resnet34.onnx";
const MODEL_BYTES: u64 = 26_534_127;
const MODEL_SHA256: &str = "9fea6516d7ad6bf0a76c7689f5a49b65d330fad6dde96c91bb4435ffbfe056a1";

const SAMPLE_RATE: f64 = 16_000.0;
/// Segments shorter than this give unreliable embeddings; they inherit a neighbour's label.
const MIN_SEGMENT_SECS: f64 = 0.5;
/// Longer segments are truncated; 20s is plenty to characterise a voice.
const MAX_SEGMENT_SECS: f64 = 20.0;
/// Cosine similarity at or above which two clusters are merged into one speaker.
/// ponytail: calibration knob. Measured with this pipeline: same voice 0.82-0.92, different
/// voices (incl. two similar male voices) 0.23-0.48 — see `real_model_separates_three_voices`.
/// 0.55 sits in that gap with margin for real-room audio, where same-speaker scores drop.
/// Raise it if distinct people get merged; lower it if one person is split into several.
const SAME_SPEAKER_THRESHOLD: f32 = 0.55;

static IN_PROGRESS: AtomicBool = AtomicBool::new(false);

struct Guard;
impl Guard {
    fn acquire() -> Result<Self, String> {
        IN_PROGRESS
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Guard)
            .map_err(|_| "Speaker identification is already running".to_string())
    }
}
impl Drop for Guard {
    fn drop(&mut self) {
        IN_PROGRESS.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Serialize)]
struct Progress<'a> {
    meeting_id: &'a str,
    stage: &'a str,
    progress_percentage: u32,
    message: &'a str,
}

fn emit_progress<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, stage: &str, pct: u32, msg: &str) {
    let _ = app.emit(
        "diarization-progress",
        Progress { meeting_id, stage, progress_percentage: pct, message: msg },
    );
}

// ---------------------------------------------------------------------------
// Features: Kaldi fbank as WeSpeaker computes it (torchaudio.compliance.kaldi.fbank with
// 80 mel bins, 25ms/10ms hamming frames, int16-scaled input, no dither), then per-segment
// cepstral mean normalisation.
// ---------------------------------------------------------------------------

const FRAME_LEN: usize = 400; // 25ms @ 16kHz
const FRAME_SHIFT: usize = 160; // 10ms
const N_FFT: usize = 512; // frame length rounded up to a power of two
const N_MELS: usize = 80;

fn mel(freq: f64) -> f64 {
    1127.0 * (1.0 + freq / 700.0).ln()
}

/// Triangular mel filters over FFT bins 0..N_FFT/2 (Kaldi drops the Nyquist bin).
fn mel_filterbank() -> Vec<Vec<f32>> {
    let (low, high) = (mel(20.0), mel(SAMPLE_RATE / 2.0));
    let delta = (high - low) / (N_MELS as f64 + 1.0);
    let bin_hz = SAMPLE_RATE / N_FFT as f64;
    (0..N_MELS)
        .map(|m| {
            let (left, center, right) = (
                low + m as f64 * delta,
                low + (m + 1) as f64 * delta,
                low + (m + 2) as f64 * delta,
            );
            (0..N_FFT / 2)
                .map(|k| {
                    let f = mel(k as f64 * bin_hz);
                    if f > left && f < right {
                        (if f <= center { (f - left) / (center - left) } else { (right - f) / (right - center) }) as f32
                    } else {
                        0.0
                    }
                })
                .collect()
        })
        .collect()
}

/// 16kHz mono samples in [-1, 1] -> CMN-normalised log-mel frames (T x 80).
pub(crate) fn fbank(samples: &[f32]) -> Vec<[f32; N_MELS]> {
    if samples.len() < FRAME_LEN {
        return Vec::new();
    }
    let n_frames = 1 + (samples.len() - FRAME_LEN) / FRAME_SHIFT;
    let window: Vec<f32> = (0..FRAME_LEN)
        .map(|i| (0.54 - 0.46 * (2.0 * std::f64::consts::PI * i as f64 / (FRAME_LEN - 1) as f64).cos()) as f32)
        .collect();
    let banks = mel_filterbank();
    let fft = realfft::RealFftPlanner::<f32>::new().plan_fft_forward(N_FFT);
    let mut buf = vec![0f32; N_FFT];
    let mut spectrum = fft.make_output_vec();

    let mut feats: Vec<[f32; N_MELS]> = Vec::with_capacity(n_frames);
    for t in 0..n_frames {
        let frame = &samples[t * FRAME_SHIFT..t * FRAME_SHIFT + FRAME_LEN];
        buf.fill(0.0);
        for (b, s) in buf.iter_mut().zip(frame) {
            *b = s * 32768.0;
        }
        let mean = buf[..FRAME_LEN].iter().sum::<f32>() / FRAME_LEN as f32;
        buf[..FRAME_LEN].iter_mut().for_each(|x| *x -= mean);
        for i in (1..FRAME_LEN).rev() {
            buf[i] -= 0.97 * buf[i - 1];
        }
        buf[0] -= 0.97 * buf[0];
        buf[..FRAME_LEN].iter_mut().zip(&window).for_each(|(x, w)| *x *= w);

        fft.process(&mut buf, &mut spectrum).expect("fixed-size FFT");
        let power: Vec<f32> = spectrum[..N_FFT / 2].iter().map(|c| c.norm_sqr()).collect();
        let mut row = [0f32; N_MELS];
        for (r, bank) in row.iter_mut().zip(&banks) {
            let energy: f32 = bank.iter().zip(&power).map(|(w, p)| w * p).sum();
            *r = energy.max(f32::EPSILON).ln();
        }
        feats.push(row);
    }

    // Cepstral mean normalisation over the segment.
    let mut means = [0f32; N_MELS];
    for row in &feats {
        means.iter_mut().zip(row).for_each(|(m, v)| *m += v);
    }
    means.iter_mut().for_each(|m| *m /= n_frames as f32);
    for row in &mut feats {
        row.iter_mut().zip(&means).for_each(|(v, m)| *v -= m);
    }
    feats
}

// ---------------------------------------------------------------------------
// Embedding model
// ---------------------------------------------------------------------------

struct Embedder {
    session: Session,
    input: String,
    output: String,
}

impl Embedder {
    fn load(path: &Path) -> Result<Self> {
        crate::ensure_onnx_runtime_available()?;
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_execution_providers(vec![CPUExecutionProvider::default().build()])?
            .commit_from_file(path)?;
        let input = session.inputs.first().ok_or_else(|| anyhow!("model has no inputs"))?.name.clone();
        let output = session.outputs.first().ok_or_else(|| anyhow!("model has no outputs"))?.name.clone();
        Ok(Self { session, input, output })
    }

    /// L2-normalised speaker embedding for one stretch of 16kHz audio.
    fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        let feats = fbank(samples);
        let frames = Array3::from_shape_vec((1, feats.len(), N_MELS), feats.concat())?;
        let outputs = self
            .session
            .run(ort::inputs![self.input.as_str() => TensorRef::from_array_view(frames.view())?])?;
        let emb = outputs
            .get(self.output.as_str())
            .ok_or_else(|| anyhow!("embedding output missing"))?
            .try_extract_array::<f32>()?;
        let mut v: Vec<f32> = emb.iter().copied().collect();
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        v.iter_mut().for_each(|x| *x /= norm);
        Ok(v)
    }
}

// ---------------------------------------------------------------------------
// Clustering: centroid-linkage agglomerative clustering on cosine similarity.
// ---------------------------------------------------------------------------

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let na = a.iter().map(|x| x * x).sum::<f32>().sqrt();
    let nb = b.iter().map(|x| x * x).sum::<f32>().sqrt();
    dot / (na * nb).max(1e-12)
}

/// Returns a cluster id per embedding, numbered 0.. in order of first appearance.
/// O(n^2) memory and O(n^2) per merge; fine for the few hundred segments of a meeting.
pub(crate) fn cluster(embeddings: &[Vec<f32>], threshold: f32) -> Vec<usize> {
    let n = embeddings.len();
    // Sum of member embeddings; cosine is scale-invariant, so the sum acts as the centroid.
    let mut sums = embeddings.to_vec();
    let mut alive = vec![true; n];
    let mut owner: Vec<usize> = (0..n).collect();
    let mut sim = vec![f32::NEG_INFINITY; n * n];
    for i in 0..n {
        for j in i + 1..n {
            sim[i * n + j] = cosine(&sums[i], &sums[j]);
        }
    }
    loop {
        let mut best = (f32::NEG_INFINITY, 0, 0);
        for i in (0..n).filter(|&i| alive[i]) {
            for j in (i + 1..n).filter(|&j| alive[j]) {
                if sim[i * n + j] > best.0 {
                    best = (sim[i * n + j], i, j);
                }
            }
        }
        let (score, keep, gone) = best;
        if score < threshold {
            break;
        }
        let merged: Vec<f32> = sums[keep].iter().zip(&sums[gone]).map(|(a, b)| a + b).collect();
        sums[keep] = merged;
        alive[gone] = false;
        owner.iter_mut().filter(|o| **o == gone).for_each(|o| *o = keep);
        for k in (0..n).filter(|&k| alive[k] && k != keep) {
            let (a, b) = (keep.min(k), keep.max(k));
            sim[a * n + b] = cosine(&sums[a], &sums[b]);
        }
    }
    // Renumber by first appearance so "Speaker 1" is whoever talks first.
    let mut ids: Vec<usize> = Vec::new();
    owner
        .iter()
        .map(|o| ids.iter().position(|x| x == o).unwrap_or_else(|| { ids.push(*o); ids.len() - 1 }))
        .collect()
}

/// Fill labels for segments that had no embedding (too short) from the nearest labelled
/// segment in time order; if nothing was labelled, everyone is speaker 0.
fn fill_gaps(labels: &mut [Option<usize>]) {
    let known: Vec<(usize, usize)> = labels.iter().enumerate().filter_map(|(i, l)| l.map(|l| (i, l))).collect();
    for (i, label) in labels.iter_mut().enumerate() {
        if label.is_none() {
            *label = Some(
                known
                    .iter()
                    .min_by_key(|(j, _)| (*j as isize - i as isize).unsigned_abs())
                    .map_or(0, |(_, l)| *l),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Model download (first use), with pinned size + SHA-256.
// ---------------------------------------------------------------------------

async fn ensure_model<R: Runtime>(app: &AppHandle<R>, meeting_id: &str) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    if let Some(org) = crate::policy::org_model_path(Path::new("diarization").join(MODEL_FILE)) {
        return Ok(org);
    }
    let dir = app.path().app_data_dir()?.join("models").join("diarization");
    let path = dir.join(MODEL_FILE);
    if tokio::fs::metadata(&path).await.map(|m| m.len() == MODEL_BYTES).unwrap_or(false) {
        return Ok(path);
    }
    crate::policy::require_downloads_allowed().map_err(|e| anyhow!(e))?;
    if crate::network_policy::is_strict_offline() {
        return Err(anyhow!(
            "The speaker model ({} MB) must be downloaded once, but Strict Offline Mode is on",
            MODEL_BYTES / 1_000_000
        ));
    }
    tokio::fs::create_dir_all(&dir).await?;
    emit_progress(app, meeting_id, "downloading", 0, "Downloading speaker model...");

    let mut response = reqwest::Client::new().get(MODEL_URL).send().await?.error_for_status()?;
    let tmp = dir.join(format!("{MODEL_FILE}.part"));
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    while let Some(chunk) = response.chunk().await? {
        received += chunk.len() as u64;
        if received > MODEL_BYTES {
            break;
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        let pct = (received * 20 / MODEL_BYTES) as u32;
        emit_progress(app, meeting_id, "downloading", pct, "Downloading speaker model...");
    }
    file.flush().await?;
    drop(file);

    let actual = format!("{:x}", hasher.finalize());
    if received != MODEL_BYTES || actual != MODEL_SHA256 {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(anyhow!(
            "Speaker model failed verification (got {} bytes, sha256 {}); not using it",
            received,
            actual
        ));
    }
    tokio::fs::rename(&tmp, &path).await?;
    info!("Speaker model downloaded and verified: {:?}", path);
    Ok(path)
}

// ---------------------------------------------------------------------------
// Orchestration
// ---------------------------------------------------------------------------

async fn identify<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, folder: &Path) -> Result<usize> {
    let model_path = ensure_model(app, meeting_id).await?;

    let pool = app
        .try_state::<AppState>()
        .ok_or_else(|| anyhow!("App state not available"))?
        .db_manager
        .pool()
        .clone();
    let rows: Vec<(String, Option<f64>, Option<f64>)> = sqlx::query_as(
        "SELECT id, audio_start_time, audio_end_time FROM transcripts
         WHERE meeting_id = ? ORDER BY audio_start_time ASC",
    )
    .bind(meeting_id)
    .fetch_all(&pool)
    .await?;
    if rows.is_empty() {
        return Err(anyhow!("This meeting has no transcript to label"));
    }

    emit_progress(app, meeting_id, "decoding", 22, "Reading recording...");
    let audio_path = crate::audio::retranscription::find_audio_file(folder)?;
    let samples = tokio::task::spawn_blocking(move || -> Result<Vec<f32>> {
        Ok(crate::audio::decoder::decode_audio_file(&audio_path)?.to_whisper_format())
    })
    .await??;

    emit_progress(app, meeting_id, "analyzing", 30, "Analyzing voices...");
    let app_for_task = app.clone();
    let meeting = meeting_id.to_string();
    let spans: Vec<Option<(f64, f64)>> = rows.iter().map(|(_, s, e)| s.zip(*e)).collect();
    let labels = tokio::task::spawn_blocking(move || -> Result<Vec<Option<usize>>> {
        let mut embedder = Embedder::load(&model_path).context("loading speaker model")?;
        let mut embeddings = Vec::new();
        let mut owners = Vec::new(); // row index of each embedding
        for (i, span) in spans.iter().enumerate() {
            let Some((start, end)) = *span else { continue };
            if end - start < MIN_SEGMENT_SECS {
                continue;
            }
            let from = ((start * SAMPLE_RATE) as usize).min(samples.len());
            let to = (((start + (end - start).min(MAX_SEGMENT_SECS)) * SAMPLE_RATE) as usize).min(samples.len());
            if to - from < (MIN_SEGMENT_SECS * SAMPLE_RATE) as usize {
                continue;
            }
            match embedder.embed(&samples[from..to]) {
                Ok(e) => {
                    embeddings.push(e);
                    owners.push(i);
                }
                Err(e) => warn!("Skipping segment {}: {}", i, e),
            }
            let pct = 30 + (i * 60 / spans.len().max(1)) as u32;
            emit_progress(&app_for_task, &meeting, "analyzing", pct, "Analyzing voices...");
        }
        let mut labels = vec![None; spans.len()];
        for (row, id) in owners.iter().zip(cluster(&embeddings, SAME_SPEAKER_THRESHOLD)) {
            labels[*row] = Some(id);
        }
        fill_gaps(&mut labels);
        Ok(labels)
    })
    .await??;

    emit_progress(app, meeting_id, "saving", 92, "Saving speakers...");
    let mut tx = pool.begin().await?;
    for ((id, _, _), label) in rows.iter().zip(&labels) {
        sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ? AND meeting_id = ?")
            .bind(format!("Speaker {}", label.unwrap_or(0) + 1))
            .bind(id)
            .bind(meeting_id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(labels.iter().flatten().max().map_or(1, |m| m + 1))
}

/// Starts speaker identification in the background. Emits `diarization-progress`, then
/// `diarization-complete {meeting_id, speakers}` or `diarization-error {meeting_id, error}`.
#[tauri::command]
pub async fn start_speaker_identification<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    meeting_folder_path: String,
) -> Result<(), String> {
    let guard = Guard::acquire()?;
    tauri::async_runtime::spawn(async move {
        let _guard = guard;
        match identify(&app, &meeting_id, Path::new(&meeting_folder_path)).await {
            Ok(speakers) => {
                info!("Identified {} speakers for meeting {}", speakers, meeting_id);
                let _ = app.emit("diarization-complete", serde_json::json!({ "meeting_id": meeting_id, "speakers": speakers }));
            }
            Err(e) => {
                warn!("Speaker identification failed for {}: {:#}", meeting_id, e);
                let _ = app.emit("diarization-error", serde_json::json!({ "meeting_id": meeting_id, "error": format!("{:#}", e) }));
            }
        }
    });
    Ok(())
}

/// Renames a speaker across the whole meeting. Renaming onto an existing name merges them.
#[tauri::command]
pub async fn api_rename_speaker<R: Runtime>(
    app: AppHandle<R>,
    meeting_id: String,
    from: String,
    to: String,
) -> Result<u64, String> {
    let to = to.trim();
    if to.is_empty() || to.chars().count() > 64 {
        return Err("Speaker name must be 1-64 characters".into());
    }
    let state = app.try_state::<AppState>().ok_or("App state not available")?;
    sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
        .bind(to)
        .bind(&meeting_id)
        .bind(&from)
        .execute(state.db_manager.pool())
        .await
        .map(|r| r.rows_affected())
        .map_err(|e| format!("Failed to rename speaker: {}", e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fbank_shape_cmn_and_pitch() {
        // 1s of a 1kHz tone.
        let tone: Vec<f32> = (0..16_000).map(|i| (2.0 * std::f32::consts::PI * 1000.0 * i as f32 / 16_000.0).sin() * 0.5).collect();
        let f = fbank(&tone);
        assert_eq!(f.len(), 1 + (16_000 - 400) / 160);
        // CMN: every column averages to ~0.
        for m in 0..N_MELS {
            let mean: f32 = f.iter().map(|r| r[m]).sum::<f32>() / f.len() as f32;
            assert!(mean.abs() < 1e-3, "column {m} mean {mean}");
        }
        // The filter that weights the 1kHz FFT bin most is centred near 1kHz.
        let banks = mel_filterbank();
        let bin_1k = (1000.0 / (SAMPLE_RATE / N_FFT as f64)).round() as usize;
        let peak = (0..N_MELS).max_by(|a, b| banks[*a][bin_1k].total_cmp(&banks[*b][bin_1k])).unwrap();
        let center_hz = 700.0 * ((mel(20.0) + (peak + 1) as f64 * (mel(8000.0) - mel(20.0)) / 81.0) / 1127.0).exp_m1();
        assert!((center_hz - 1000.0).abs() < 60.0, "1kHz lands in bank centred at {center_hz}");
        assert!(fbank(&tone[..399]).is_empty());
    }

    /// End-to-end check with the real model. Needs DIARIZATION_TEST_DIR containing
    /// voxceleb_resnet34.onnx and v{0,1,2}_l{0,1,2}.wav (three voices x three lines, 16kHz).
    #[test]
    #[ignore]
    fn real_model_separates_three_voices() {
        let dir = PathBuf::from(std::env::var("DIARIZATION_TEST_DIR").expect("DIARIZATION_TEST_DIR"));
        let mut embedder = Embedder::load(&dir.join(MODEL_FILE)).unwrap();
        // Interleaved like a conversation: v0, v1, v2, v0, v1, v2, ...
        let order: Vec<(usize, usize)> = (0..3).flat_map(|l| (0..3).map(move |v| (v, l))).collect();
        let embs: Vec<Vec<f32>> = order
            .iter()
            .map(|(v, l)| {
                let audio = crate::audio::decoder::decode_audio_file(&dir.join(format!("v{v}_l{l}.wav"))).unwrap();
                embedder.embed(&audio.to_whisper_format()).unwrap()
            })
            .collect();
        for (i, a) in embs.iter().enumerate() {
            let row: Vec<String> = embs.iter().map(|b| format!("{:5.2}", cosine(a, b))).collect();
            println!("v{} {}", order[i].0, row.join(" "));
        }
        assert_eq!(cluster(&embs, SAME_SPEAKER_THRESHOLD), vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn clusters_by_speaker_and_numbers_by_first_appearance() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let a2 = vec![0.95, 0.1, 0.0];
        let b2 = vec![0.1, 0.9, 0.05];
        assert_eq!(cluster(&[b.clone(), a.clone(), b2, a2], 0.3), vec![0, 1, 0, 1]);
        assert_eq!(cluster(&[a.clone(), b.clone()], 0.3), vec![0, 1]);
        assert_eq!(cluster(&[a.clone(), a.clone(), a], 0.3), vec![0, 0, 0]);
        assert!(cluster(&[], 0.3).is_empty());

        let mut labels = vec![None, Some(1), None, None, Some(0)];
        fill_gaps(&mut labels);
        assert_eq!(labels, vec![Some(1), Some(1), Some(1), Some(0), Some(0)]);
        let mut none = vec![None, None];
        fill_gaps(&mut none);
        assert_eq!(none, vec![Some(0), Some(0)]);
    }
}
