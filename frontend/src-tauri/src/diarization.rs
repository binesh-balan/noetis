//! Post-recording speaker identification ("who said what"), fully on-device, following the
//! pyannote 3.1 recipe:
//!
//! 1. **Segmentation** — pyannote segmentation-3.0 (ONNX, MIT) over sliding 10 s windows gives,
//!    every ~17 ms, which of up to 3 local speakers is talking (overlap included).
//! 2. **Embedding** — for each local speaker in each window, a WeSpeaker ResNet34 embedding
//!    (ONNX, Apache-2.0, Kaldi fbank input) of the frames where only they speak.
//! 3. **Clustering** — embeddings are clustered across the meeting, so each local speaker maps
//!    to a global "Speaker N".
//! 4. **Timeline** — window activity is aggregated into one global frame-level speaker track.
//! 5. **Lines** — transcript lines are cut by pauses and often span a turn change, so each line
//!    is split where the dominant speaker changes, using Parakeet word timestamps to cut the
//!    text (even time split as fallback). Renaming/merging speakers is a plain UPDATE.

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

/// A pinned model file: HuggingFace URL at a fixed commit, exact size, SHA-256 (git-lfs oid).
struct ModelFile {
    file: &'static str,
    url: &'static str,
    bytes: u64,
    sha256: &'static str,
}

const EMBEDDING_MODEL: ModelFile = ModelFile {
    file: "voxceleb_resnet34.onnx",
    url: "https://huggingface.co/Wespeaker/wespeaker-voxceleb-resnet34/resolve/ff1ac5bca8ef11e90662b879aa923979e0bd277b/voxceleb_resnet34.onnx",
    bytes: 26_534_127,
    sha256: "9fea6516d7ad6bf0a76c7689f5a49b65d330fad6dde96c91bb4435ffbfe056a1",
};

const SEGMENTATION_MODEL: ModelFile = ModelFile {
    file: "segmentation-3.0.onnx",
    url: "https://huggingface.co/onnx-community/pyannote-segmentation-3.0/resolve/733a93b6473d019a773298e08cefa686894b1854/onnx/model.onnx",
    bytes: 5_986_908,
    sha256: "057ee564753071c0b09b5b611648b50ac188d50846bff5f01e9f7bbf1591ea25",
};

const SAMPLE_RATE: f64 = 16_000.0;
/// Segmentation window and hop (pyannote trains on 10 s chunks).
const SEG_WINDOW_SECS: f64 = 10.0;
const SEG_STEP_SECS: f64 = 2.5;
/// segmentation-3.0 emits one frame per 270 samples; its receptive field is 990 samples.
const FRAME_STEP_SAMPLES: usize = 270;
const FRAME_SECS: f64 = FRAME_STEP_SAMPLES as f64 / SAMPLE_RATE;
/// Minimum clean (non-overlapped) speech of a local speaker to trust its embedding.
const MIN_EMBED_SECS: f64 = 0.5;
/// Cosine similarity at or above which two clusters are merged into one speaker.
/// ponytail: calibration knob. On the AMI ES2004a meeting (4 real speakers, human labels)
/// every value in 0.30-0.45 finds exactly 4 speakers with 79.6% of speech attributed
/// correctly; 0.50+ over-splits (6-8 speakers). 0.40 is the middle of that stable band.
/// Re-check with `real_meeting_folder` (DIAR_THRESHOLDS) before changing it.
const SAME_SPEAKER_THRESHOLD: f32 = 0.40;
/// A "speaker" with less total speech than this is folded into the most similar real speaker.
const MIN_SPEAKER_SECS: f64 = 5.0;
/// Embeddings clustered directly; the rest go to the nearest resulting speaker. Bounds the
/// O(n^2)-per-merge agglomerative step for long meetings.
const MAX_CLUSTERED: usize = 800;
/// Turns shorter than this inside a line are treated as noise and absorbed (avoids
/// one-word fragments like "you" split off a sentence).
const MIN_TURN_SECS: f64 = 1.0;

/// Tunables, overridable in tests for calibration sweeps.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Params {
    pub threshold: f32,
    pub min_embed_secs: f64,
    pub min_turn_secs: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self { threshold: SAME_SPEAKER_THRESHOLD, min_embed_secs: MIN_EMBED_SECS, min_turn_secs: MIN_TURN_SECS }
    }
}

/// Local speaker activity per powerset class: none, 1, 2, 3, 1+2, 1+3, 2+3.
const POWERSET: [[bool; 3]; 7] = [
    [false, false, false],
    [true, false, false],
    [false, true, false],
    [false, false, true],
    [true, true, false],
    [true, false, true],
    [false, true, true],
];

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
// ONNX models
// ---------------------------------------------------------------------------

/// A single-input, single-output ONNX session.
struct OnnxModel {
    session: Session,
    input: String,
    output: String,
}

impl OnnxModel {
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

    fn run(&mut self, input: &Array3<f32>) -> Result<(Vec<usize>, Vec<f32>)> {
        let outputs = self
            .session
            .run(ort::inputs![self.input.as_str() => TensorRef::from_array_view(input.view())?])?;
        let out = outputs
            .get(self.output.as_str())
            .ok_or_else(|| anyhow!("model output missing"))?
            .try_extract_array::<f32>()?;
        Ok((out.shape().to_vec(), out.iter().copied().collect()))
    }
}

/// WeSpeaker ResNet34: audio -> L2-normalised 256-d speaker embedding.
struct Embedder(OnnxModel);

impl Embedder {
    fn load(path: &Path) -> Result<Self> {
        OnnxModel::load(path).map(Self)
    }

    fn embed(&mut self, samples: &[f32]) -> Result<Vec<f32>> {
        let feats = fbank(samples);
        let frames = Array3::from_shape_vec((1, feats.len(), N_MELS), feats.concat())?;
        let (_, mut v) = self.0.run(&frames)?;
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        v.iter_mut().for_each(|x| *x /= norm);
        Ok(v)
    }
}

/// pyannote segmentation-3.0: a 10 s chunk -> per-frame activity of 3 local speakers.
struct Segmenter(OnnxModel);

impl Segmenter {
    fn load(path: &Path) -> Result<Self> {
        OnnxModel::load(path).map(Self)
    }

    fn run(&mut self, chunk: &[f32]) -> Result<Vec<[bool; 3]>> {
        let n = (SEG_WINDOW_SECS * SAMPLE_RATE) as usize;
        let mut buf = vec![0f32; n];
        buf[..chunk.len().min(n)].copy_from_slice(&chunk[..chunk.len().min(n)]);
        let (shape, logits) = self.0.run(&Array3::from_shape_vec((1, 1, n), buf)?)?;
        let classes = *shape.last().ok_or_else(|| anyhow!("bad segmentation output"))?;
        if classes != POWERSET.len() {
            return Err(anyhow!("unexpected segmentation output with {classes} classes"));
        }
        Ok(logits
            .chunks(classes)
            .map(|frame| {
                let best = (0..classes).max_by(|&a, &b| frame[a].total_cmp(&frame[b])).unwrap_or(0);
                POWERSET[best]
            })
            .collect())
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

fn renumber_by_first_appearance(ids: &[usize]) -> Vec<usize> {
    let mut order: Vec<usize> = Vec::new();
    ids.iter()
        .map(|o| order.iter().position(|x| x == o).unwrap_or_else(|| { order.push(*o); order.len() - 1 }))
        .collect()
}

/// Returns a cluster id per embedding, numbered 0.. in order of first appearance.
/// O(n^2) memory and O(n^2) per merge; callers bound n (see MAX_CLUSTERED).
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
    renumber_by_first_appearance(&owner)
}

/// Cluster, then fold low-speech clusters into their most similar substantial cluster.
/// `durations` = seconds of speech behind each embedding. Ids renumbered by first appearance.
pub(crate) fn assign_speakers(embeddings: &[Vec<f32>], durations: &[f64], threshold: f32) -> Vec<usize> {
    let mut ids = cluster(embeddings, threshold);
    let k = ids.iter().max().map_or(0, |m| m + 1);
    let speech: Vec<f64> = (0..k)
        .map(|c| ids.iter().zip(durations).filter(|(i, _)| **i == c).map(|(_, d)| d).sum())
        .collect();
    let big: Vec<usize> = (0..k).filter(|&c| speech[c] >= MIN_SPEAKER_SECS).collect();
    if big.is_empty() || big.len() == k {
        return ids;
    }
    let centroid = |c: usize, ids: &[usize]| -> Vec<f32> {
        let mut sum = vec![0f32; embeddings[0].len()];
        for (e, _) in embeddings.iter().zip(ids).filter(|(_, i)| **i == c) {
            sum.iter_mut().zip(e).for_each(|(s, x)| *s += x);
        }
        sum
    };
    let big_centroids: Vec<(usize, Vec<f32>)> = big.iter().map(|&b| (b, centroid(b, &ids))).collect();
    for small in (0..k).filter(|c| !big.contains(c)) {
        let c = centroid(small, &ids);
        let target = big_centroids
            .iter()
            .max_by(|a, b| cosine(&c, &a.1).total_cmp(&cosine(&c, &b.1)))
            .map(|(b, _)| *b)
            .unwrap();
        ids.iter_mut().filter(|i| **i == small).for_each(|i| *i = target);
    }
    renumber_by_first_appearance(&ids)
}

/// Like [`assign_speakers`] but for any number of embeddings: cluster an evenly spaced
/// subsample (at most MAX_CLUSTERED), then give every embedding the nearest centroid.
fn assign_speakers_scaled(embeddings: &[Vec<f32>], durations: &[f64], threshold: f32) -> Vec<usize> {
    if embeddings.is_empty() {
        return Vec::new();
    }
    let stride = embeddings.len().div_ceil(MAX_CLUSTERED).max(1);
    let sample: Vec<Vec<f32>> = embeddings.iter().step_by(stride).cloned().collect();
    let sample_durs: Vec<f64> = durations.chunks(stride).map(|c| c.iter().sum()).collect();
    let ids = assign_speakers(&sample, &sample_durs, threshold);
    if stride == 1 {
        return ids;
    }
    let k = ids.iter().max().map_or(0, |m| m + 1);
    let mut centroids = vec![vec![0f32; embeddings[0].len()]; k];
    for (e, &c) in sample.iter().zip(&ids) {
        centroids[c].iter_mut().zip(e).for_each(|(s, x)| *s += x);
    }
    let nearest: Vec<usize> = embeddings
        .iter()
        .map(|e| (0..k).max_by(|&a, &b| cosine(e, &centroids[a]).total_cmp(&cosine(e, &centroids[b]))).unwrap_or(0))
        .collect();
    renumber_by_first_appearance(&nearest)
}

// ---------------------------------------------------------------------------
// Diarization: segmentation + embeddings + clustering -> frame-level speaker track.
// ---------------------------------------------------------------------------

/// Start offsets (in samples) of the 10 s segmentation windows covering `len` samples.
fn window_starts(len: usize) -> Vec<usize> {
    let (win, step) = ((SEG_WINDOW_SECS * SAMPLE_RATE) as usize, (SEG_STEP_SECS * SAMPLE_RATE) as usize);
    if len <= win {
        return vec![0];
    }
    let mut starts: Vec<usize> = (0..=len - win).step_by(step).collect();
    if starts.last() != Some(&(len - win)) {
        starts.push(len - win);
    }
    starts
}

/// Output of the expensive stage: segmentation activity per window and the embeddings of
/// each window's local speakers. Clustering/aggregation (cheap) is done by [`Extracted::resolve`].
pub(crate) struct Extracted {
    starts: Vec<usize>,
    activity: Vec<Vec<[bool; 3]>>,
    embeddings: Vec<Vec<f32>>,
    durations: Vec<f64>,
    owners: Vec<(usize, usize)>, // (window, local speaker) per embedding
    total_frames: usize,
}

fn extract(
    segmenter: &mut Segmenter,
    embedder: &mut Embedder,
    audio: &[f32],
    params: &Params,
    mut progress: impl FnMut(f64),
) -> Result<Extracted> {
    let starts = window_starts(audio.len());
    let mut x = Extracted {
        activity: Vec::with_capacity(starts.len()),
        embeddings: Vec::new(),
        durations: Vec::new(),
        owners: Vec::new(),
        total_frames: audio.len().div_ceil(FRAME_STEP_SAMPLES),
        starts,
    };
    for w in 0..x.starts.len() {
        let start = x.starts[w];
        let end = (start + (SEG_WINDOW_SECS * SAMPLE_RATE) as usize).min(audio.len());
        let act = segmenter.run(&audio[start..end])?;
        for local in 0..3 {
            // Frames where only this local speaker talks: clean audio for its embedding.
            let mut clean = Vec::new();
            for (f, a) in act.iter().enumerate() {
                if a[local] && a.iter().filter(|v| **v).count() == 1 {
                    let s = (start + f * FRAME_STEP_SAMPLES).min(end);
                    clean.extend_from_slice(&audio[s..(s + FRAME_STEP_SAMPLES).min(end)]);
                }
            }
            let secs = clean.len() as f64 / SAMPLE_RATE;
            if secs >= params.min_embed_secs {
                match embedder.embed(&clean) {
                    Ok(e) => { x.embeddings.push(e); x.durations.push(secs); x.owners.push((w, local)); }
                    Err(e) => warn!("Skipping local speaker {local} in window {w}: {e}"),
                }
            }
        }
        x.activity.push(act);
        progress((w + 1) as f64 / x.starts.len() as f64);
    }
    Ok(x)
}

/// Frame-level speaker track plus each speaker's centroid embedding (L2-normalised, indexed by id).
pub(crate) struct Diarization {
    pub track: Vec<Option<usize>>,
    pub centroids: Vec<Vec<f32>>,
}

/// Seconds of speech per speaker id in a frame track.
pub(crate) fn speech_secs(track: &[Option<usize>], k: usize) -> Vec<f64> {
    let mut secs = vec![0.0; k];
    for id in track.iter().flatten() {
        if *id < k {
            secs[*id] += FRAME_SECS;
        }
    }
    secs
}

impl Extracted {
    /// Global speaker (or None for silence) for every FRAME_SECS frame of the audio, plus
    /// each speaker's centroid embedding.
    fn resolve(&self, threshold: f32) -> Diarization {
        let ids = assign_speakers_scaled(&self.embeddings, &self.durations, threshold);
        let k = ids.iter().max().map_or(0, |m| m + 1).max(1);
        let mut centroids = vec![vec![0f32; self.embeddings.first().map_or(0, |e| e.len())]; k];
        for (e, &id) in self.embeddings.iter().zip(&ids) {
            centroids[id].iter_mut().zip(e).for_each(|(a, b)| *a += b);
        }
        let centroids = centroids.into_iter().map(crate::voices::normalize).collect();
        let mut global = vec![[None::<usize>; 3]; self.starts.len()];
        for (&(w, local), &id) in self.owners.iter().zip(&ids) {
            global[w][local] = Some(id);
        }
        // Average each speaker's activity over all windows covering a frame; the most active
        // speaker wins if it is active in at least half of those windows.
        let mut score = vec![0f32; self.total_frames * k];
        let mut cover = vec![0u16; self.total_frames];
        for (w, &start) in self.starts.iter().enumerate() {
            let offset = start / FRAME_STEP_SAMPLES;
            for (f, a) in self.activity[w].iter().enumerate() {
                let g = offset + f;
                if g >= self.total_frames {
                    break;
                }
                cover[g] += 1;
                for local in 0..3 {
                    if let (true, Some(id)) = (a[local], global[w][local]) {
                        score[g * k + id] += 1.0;
                    }
                }
            }
        }
        let track = (0..self.total_frames)
            .map(|g| {
                let row = &score[g * k..g * k + k];
                (0..k)
                    .max_by(|&a, &b| row[a].total_cmp(&row[b]))
                    .filter(|&best| cover[g] > 0 && row[best] > 0.0 && row[best] >= 0.5 * cover[g] as f32)
            })
            .collect();
        Diarization { track, centroids }
    }
}

/// Global speaker (or None for silence) for every FRAME_SECS frame of `audio`, plus centroids.
fn diarize(
    segmenter: &mut Segmenter,
    embedder: &mut Embedder,
    audio: &[f32],
    params: &Params,
    progress: impl FnMut(f64),
) -> Result<Diarization> {
    Ok(extract(segmenter, embedder, audio, params, progress)?.resolve(params.threshold))
}

/// Turns within one line (start, end in seconds): runs of the dominant speaker on the frame
/// track, with runs shorter than MIN_TURN_SECS absorbed into a neighbour. Returns
/// (speaker, start, end) covering exactly the line, or empty if nobody was detected.
pub(crate) fn line_turns(track: &[Option<usize>], span: (f64, f64), min_turn_secs: f64) -> Vec<(usize, f64, f64)> {
    let (from, to) = ((span.0 / FRAME_SECS) as usize, ((span.1 / FRAME_SECS) as usize).min(track.len()));
    // (speaker, frames, first frame, last frame)
    let mut runs: Vec<(usize, usize, usize, usize)> = Vec::new();
    for f in from..to {
        let Some(s) = track[f] else { continue };
        match runs.last_mut() {
            Some(r) if r.0 == s => { r.1 += 1; r.3 = f; }
            _ => runs.push((s, 1, f, f)),
        }
    }
    let min_frames = (min_turn_secs / FRAME_SECS) as usize;
    loop {
        let short = (runs.len() > 1).then(|| runs.iter().position(|r| r.1 < min_frames)).flatten();
        let Some(i) = short else { break };
        let small = runs.remove(i);
        // Absorb into the neighbour with more speech.
        let target = match (i.checked_sub(1), (i < runs.len()).then_some(i)) {
            (Some(p), Some(n)) => if runs[p].1 >= runs[n].1 { p } else { n },
            (Some(p), None) => p,
            (None, Some(n)) => n,
            (None, None) => unreachable!(),
        };
        let t = &mut runs[target];
        t.1 += small.1;
        t.2 = t.2.min(small.2);
        t.3 = t.3.max(small.3);
        let mut j = 1;
        while j < runs.len() {
            if runs[j].0 == runs[j - 1].0 {
                let r = runs.remove(j);
                runs[j - 1].1 += r.1;
                runs[j - 1].3 = r.3;
            } else {
                j += 1;
            }
        }
    }
    let n = runs.len();
    runs.iter()
        .enumerate()
        .map(|(i, r)| {
            let start = if i == 0 { span.0 } else { ((runs[i - 1].3 + r.2 + 1) as f64 / 2.0) * FRAME_SECS };
            let end = if i + 1 == n { span.1 } else { ((r.3 + runs[i + 1].2 + 1) as f64 / 2.0) * FRAME_SECS };
            (r.0, start, end)
        })
        .collect()
}

/// Words with start times (seconds from the start of the audio passed to Parakeet).
fn words_from_tokens(r: &crate::parakeet_engine::TimestampedResult) -> Vec<(String, f64)> {
    words_from_pieces(r.tokens.iter().map(String::as_str).zip(r.timestamps.iter().map(|&t| t as f64)))
}

/// Joins sub-word pieces into words. A piece starts a new word when it begins with the
/// SentencePiece marker or a space (ParakeetModel already maps the marker to a space).
fn words_from_pieces<'a>(pieces: impl Iterator<Item = (&'a str, f64)>) -> Vec<(String, f64)> {
    let mut words: Vec<(String, f64)> = Vec::new();
    for (tok, t) in pieces {
        let starts_word = tok.starts_with('\u{2581}') || tok.starts_with(' ');
        let piece = tok.trim_start_matches(['\u{2581}', ' ']);
        match words.last_mut() {
            Some(w) if !starts_word => w.0.push_str(piece),
            _ => words.push((piece.to_string(), t)),
        }
    }
    words.retain(|w| !w.0.is_empty());
    words
}

// ponytail: fallback when word timestamps aren't available (e.g. Whisper provider): spreads
// words evenly over the line, so a cut can be off by a word or two.
fn words_evenly(text: &str, duration: f64) -> Vec<(String, f64)> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let step = duration / words.len().max(1) as f64;
    words.iter().enumerate().map(|(i, w)| (w.to_string(), i as f64 * step)).collect()
}

/// Text per turn: words whose start (line_start + t) falls inside the turn.
fn split_words(words: &[(String, f64)], line_start: f64, turns: &[(usize, f64, f64)]) -> Vec<String> {
    turns
        .iter()
        .enumerate()
        .map(|(i, &(_, a, b))| {
            let last = i + 1 == turns.len();
            words
                .iter()
                .filter(|(_, t)| {
                    let at = line_start + t;
                    (i == 0 || at >= a) && (last || at < b)
                })
                .map(|(w, _)| w.as_str())
                .collect::<Vec<_>>()
                .join(" ")
        })
        .collect()
}

/// Drop turns that got no words, merging their time into a neighbour; merge same-speaker
/// neighbours. Returns the surviving (turn, text) pairs.
fn merge_empty_turns(turns: &[(usize, f64, f64)], texts: Vec<String>) -> Vec<((usize, f64, f64), String)> {
    let mut merged: Vec<((usize, f64, f64), String)> = Vec::new();
    for (turn, text) in turns.iter().zip(texts) {
        match merged.last_mut() {
            Some(prev) if text.is_empty() || prev.0 .0 == turn.0 => {
                prev.0 .2 = turn.2;
                if !text.is_empty() {
                    prev.1 = format!("{} {}", prev.1, text);
                }
            }
            None if text.is_empty() => {}
            _ => merged.push((*turn, text)),
        }
    }
    if let (Some(first), Some(turn)) = (merged.first_mut(), turns.first()) {
        first.0 .1 = turn.1; // a dropped leading turn's time belongs to the first survivor
    }
    merged
}

/// Fill labels for lines that had no detected speech from the nearest labelled line in
/// time order; if nothing was labelled, everyone is speaker 0.
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

async fn ensure_model<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, m: &ModelFile) -> Result<PathBuf> {
    use sha2::{Digest, Sha256};
    use tokio::io::AsyncWriteExt;

    if let Some(org) = crate::policy::org_model_path(Path::new("diarization").join(m.file)) {
        return Ok(org);
    }
    let dir = app.path().app_data_dir()?.join("models").join("diarization");
    let path = dir.join(m.file);
    if tokio::fs::metadata(&path).await.map(|md| md.len() == m.bytes).unwrap_or(false) {
        return Ok(path);
    }
    crate::policy::require_downloads_allowed().map_err(|e| anyhow!(e))?;
    if crate::network_policy::is_strict_offline() {
        return Err(anyhow!(
            "The speaker models must be downloaded once, but Strict Offline Mode is on"
        ));
    }
    tokio::fs::create_dir_all(&dir).await?;
    emit_progress(app, meeting_id, "downloading", 0, "Downloading speaker models...");

    let mut response = reqwest::Client::new().get(m.url).send().await?.error_for_status()?;
    let tmp = dir.join(format!("{}.part", m.file));
    let mut file = tokio::fs::File::create(&tmp).await?;
    let mut hasher = Sha256::new();
    let mut received: u64 = 0;
    while let Some(chunk) = response.chunk().await? {
        received += chunk.len() as u64;
        if received > m.bytes {
            break;
        }
        hasher.update(&chunk);
        file.write_all(&chunk).await?;
        let pct = (received * 20 / m.bytes) as u32;
        emit_progress(app, meeting_id, "downloading", pct, "Downloading speaker models...");
    }
    file.flush().await?;
    drop(file);

    let actual = format!("{:x}", hasher.finalize());
    if received != m.bytes || actual != m.sha256 {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(anyhow!(
            "Speaker model {} failed verification (got {} bytes, sha256 {}); not using it",
            m.file,
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

/// The Parakeet engine with a model loaded, loading the configured one if needed.
/// Returns (engine, loaded_by_us). None when Parakeet isn't usable (e.g. Whisper-only setups).
async fn parakeet_for_word_times(
    pool: &sqlx::SqlitePool,
) -> Option<(std::sync::Arc<crate::parakeet_engine::ParakeetEngine>, bool)> {
    let engine = crate::parakeet_engine::commands::PARAKEET_ENGINE.lock().ok()?.clone()?;
    if engine.is_model_loaded().await {
        return Some((engine, false));
    }
    let model = match crate::policy::managed_transcription() {
        Some(t) if t.provider == "parakeet" => t.model,
        Some(_) => return None,
        None => {
            let row: Option<(String, String)> =
                sqlx::query_as("SELECT provider, model FROM transcript_settings WHERE id = '1'")
                    .fetch_optional(pool)
                    .await
                    .ok()
                    .flatten();
            match row {
                Some((p, m)) if p == "parakeet" => m,
                _ => crate::config::DEFAULT_PARAKEET_MODEL.to_string(),
            }
        }
    };
    engine.load_model(&model).await.ok()?;
    Some((engine, true))
}

type Row = (String, Option<f64>, Option<f64>, String, String);

async fn identify<R: Runtime>(app: &AppHandle<R>, meeting_id: &str, folder: &Path) -> Result<usize> {
    let embedding_path = ensure_model(app, meeting_id, &EMBEDDING_MODEL).await?;
    let segmentation_path = ensure_model(app, meeting_id, &SEGMENTATION_MODEL).await?;

    let pool = app
        .try_state::<AppState>()
        .ok_or_else(|| anyhow!("App state not available"))?
        .db_manager
        .pool()
        .clone();
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT id, audio_start_time, audio_end_time, transcript, timestamp FROM transcripts
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
    let samples = std::sync::Arc::new(
        tokio::task::spawn_blocking(move || -> Result<Vec<f32>> {
            Ok(crate::audio::decoder::decode_audio_file(&audio_path)?.to_whisper_format())
        })
        .await??,
    );

    emit_progress(app, meeting_id, "analyzing", 25, "Analyzing voices...");
    let (app_for_task, meeting, audio) = (app.clone(), meeting_id.to_string(), samples.clone());
    let Diarization { track, centroids } = tokio::task::spawn_blocking(move || -> Result<Diarization> {
        let mut segmenter = Segmenter::load(&segmentation_path).context("loading segmentation model")?;
        let mut embedder = Embedder::load(&embedding_path).context("loading speaker model")?;
        let mut last = 0u32;
        diarize(&mut segmenter, &mut embedder, &audio, &Params::default(), |done| {
            let pct = 25 + (done * 60.0) as u32;
            if pct != last {
                last = pct;
                emit_progress(&app_for_task, &meeting, "analyzing", pct, "Analyzing voices...");
            }
        })
    })
    .await??;
    let k = centroids.len();
    let secs = speech_secs(&track, k);
    // Meeting-app hints first, then known voices.
    let hints = crate::speaker_hints::load(folder);
    let mut names = crate::speaker_hints::names_from_hints(&track, k, FRAME_SECS, &hints);
    match crate::voices::load_profiles(&pool, meeting_id).await {
        Ok(profiles) => crate::voices::match_voices(&centroids, &mut names, &profiles, crate::voices::VOICE_MATCH_THRESHOLD),
        Err(e) => warn!("Voice memory unavailable for {}: {}", meeting_id, e),
    }
    let labels = crate::voices::labels(&names);

    // Turns per line; lines without detected speech inherit the nearest line's speaker.
    let mut turns: Vec<Vec<(usize, f64, f64)>> =
        rows.iter().map(|r| r.1.zip(r.2).map_or(Vec::new(), |span| line_turns(&track, span, MIN_TURN_SECS))).collect();
    let mut first: Vec<Option<usize>> = turns.iter().map(|t| t.first().map(|x| x.0)).collect();
    fill_gaps(&mut first);

    // Split the text of lines that contain a turn change.
    let mut split_text: Vec<Option<Vec<String>>> = vec![None; rows.len()];
    if turns.iter().any(|t| t.len() > 1) {
        emit_progress(app, meeting_id, "splitting", 88, "Separating speaker turns...");
        let parakeet = parakeet_for_word_times(&pool).await;
        for (i, t) in turns.iter_mut().enumerate() {
            if t.len() < 2 {
                continue;
            }
            let (_, Some(start), Some(end), text, _) = &rows[i] else { continue };
            let from = ((start * SAMPLE_RATE) as usize).min(samples.len());
            let to = ((end * SAMPLE_RATE) as usize).min(samples.len());
            let words = match &parakeet {
                Some((engine, _)) => match engine.transcribe_audio_timestamped(samples[from..to].to_vec()).await {
                    Ok(r) if !r.tokens.is_empty() => words_from_tokens(&r),
                    _ => words_evenly(text, end - start),
                },
                None => words_evenly(text, end - start),
            };
            let merged = merge_empty_turns(t, split_words(&words, *start, t));
            if merged.len() > 1 {
                *t = merged.iter().map(|m| m.0).collect();
                split_text[i] = Some(merged.into_iter().map(|m| m.1).collect());
            } else if let Some(m) = merged.first() {
                first[i] = Some(m.0 .0);
            }
        }
        if let Some((engine, true)) = &parakeet {
            engine.unload_model().await;
        }
    }

    emit_progress(app, meeting_id, "saving", 94, "Saving speakers...");
    let speaker = |id: usize| labels.get(id).cloned().unwrap_or_else(|| format!("Speaker {}", id + 1));
    let mut tx = pool.begin().await?;
    for (i, (id, _, _, _, timestamp)) in rows.iter().enumerate() {
        if let Some(texts) = &split_text[i] {
            for (k, ((spk, a, b), text)) in turns[i].iter().zip(texts).enumerate() {
                if k == 0 {
                    sqlx::query("UPDATE transcripts SET transcript = ?, audio_end_time = ?, duration = ?, speaker = ? WHERE id = ? AND meeting_id = ?")
                        .bind(text).bind(b).bind(b - a).bind(speaker(*spk)).bind(id).bind(meeting_id)
                        .execute(&mut *tx).await?;
                } else {
                    sqlx::query("INSERT INTO transcripts (id, meeting_id, transcript, timestamp, audio_start_time, audio_end_time, duration, speaker) VALUES (?, ?, ?, ?, ?, ?, ?, ?)")
                        .bind(format!("{}-t{}", id, k)).bind(meeting_id).bind(text).bind(timestamp)
                        .bind(a).bind(b).bind(b - a).bind(speaker(*spk))
                        .execute(&mut *tx).await?;
                }
            }
        } else {
            sqlx::query("UPDATE transcripts SET speaker = ? WHERE id = ? AND meeting_id = ?")
                .bind(speaker(first[i].unwrap_or(0))).bind(id).bind(meeting_id)
                .execute(&mut *tx).await?;
        }
    }

    // Voiceprints go in the labels' transaction, so a rename can't land between the two; under
    // a savepoint, so a sample failure rolls back only the samples, never the labels.
    let samples: Vec<crate::voices::Sample> = (0..k)
        .filter(|&id| secs[id] >= crate::voices::MIN_SAMPLE_SECS && !centroids[id].is_empty())
        .map(|id| crate::voices::Sample { label: labels[id].clone(), embedding: centroids[id].clone(), speech_secs: secs[id] })
        .collect();
    let saved = async {
        let mut sp = sqlx::Connection::begin(&mut *tx).await?; // nested: a SAVEPOINT
        crate::voices::save_meeting_samples(&mut sp, meeting_id, &samples).await?;
        sp.commit().await
    }
    .await;
    if let Err(e) = saved {
        warn!("Couldn't save voice samples for {}: {}", meeting_id, e);
    }
    tx.commit().await?;

    let mut all: Vec<usize> = split_text
        .iter()
        .zip(&turns)
        .zip(&first)
        .flat_map(|((split, t), f)| if split.is_some() { t.iter().map(|x| x.0).collect() } else { f.iter().copied().collect::<Vec<_>>() })
        .collect();
    all.sort_unstable();
    all.dedup();
    Ok(all.len().max(1))
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
    let fail = |e: sqlx::Error| format!("Failed to rename speaker: {}", e);
    let mut tx = state.db_manager.pool().begin().await.map_err(fail)?;
    let n = sqlx::query("UPDATE transcripts SET speaker = ? WHERE meeting_id = ? AND speaker = ?")
        .bind(to).bind(&meeting_id).bind(&from)
        .execute(&mut *tx).await.map_err(fail)?
        .rows_affected();
    // Teaches voice memory: the meeting's sample for `from` now belongs to `to`.
    crate::voices::relabel_in_meeting(&mut tx, &meeting_id, &from, to).await.map_err(fail)?;
    tx.commit().await.map_err(fail)?;
    Ok(n)
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

    #[test]
    fn clusters_by_speaker_and_folds_small_ones() {
        let a = vec![1.0, 0.0, 0.0];
        let b = vec![0.0, 1.0, 0.0];
        let a2 = vec![0.95, 0.1, 0.0];
        let b2 = vec![0.1, 0.9, 0.05];
        assert_eq!(cluster(&[b.clone(), a.clone(), b2, a2], 0.3), vec![0, 1, 0, 1]);
        assert_eq!(cluster(&[a.clone(), a.clone(), a.clone()], 0.3), vec![0, 0, 0]);
        assert!(cluster(&[], 0.3).is_empty());
        // A 2 s interjection that looks like nobody else is folded into its closest real speaker.
        let odd = vec![0.6, 0.0, 0.8];
        assert_eq!(assign_speakers(&[a.clone(), b.clone(), odd, a.clone()], &[6.0, 6.0, 2.0, 6.0], 0.9), vec![0, 1, 0, 0]);
        // The scaled variant agrees on small inputs.
        assert_eq!(assign_speakers_scaled(&[a.clone(), b.clone(), a.clone()], &[6.0; 3], 0.5), vec![0, 1, 0]);
    }

    #[test]
    fn windows_cover_the_audio() {
        let sr = SAMPLE_RATE as usize;
        assert_eq!(window_starts(5 * sr), vec![0]);
        assert_eq!(window_starts(10 * sr), vec![0]);
        let s = window_starts(21 * sr);
        assert_eq!(s.first(), Some(&0));
        assert_eq!(*s.last().unwrap(), 11 * sr, "last window ends at the end of the audio");
    }

    #[test]
    fn turns_split_lines_and_ignore_blips() {
        let f = |secs: f64| (secs / FRAME_SECS) as usize;
        let mut track = vec![None; f(10.0)];
        track[f(0.0)..f(4.0)].iter_mut().for_each(|x| *x = Some(0));
        track[f(4.0)..f(4.2)].iter_mut().for_each(|x| *x = Some(2)); // 0.2 s blip
        track[f(4.2)..f(6.0)].iter_mut().for_each(|x| *x = Some(0));
        track[f(6.0)..f(10.0)].iter_mut().for_each(|x| *x = Some(1));
        let t = line_turns(&track, (0.0, 10.0), 0.6);
        assert_eq!(t.len(), 2, "{t:?}");
        assert_eq!((t[0].0, t[1].0), (0, 1));
        assert!((t[0].2 - 6.0).abs() < 0.05 && t[0].2 == t[1].1);
        assert_eq!((t[0].1, t[1].2), (0.0, 10.0));
        assert!(line_turns(&vec![None; 100], (0.0, 1.0), 0.6).is_empty());

        let words: Vec<(String, f64)> = [("I'd", 0.1), ("rather", 1.0), ("be", 3.0), ("What", 6.5), ("is", 7.0)]
            .iter().map(|(w, t)| (w.to_string(), *t)).collect();
        assert_eq!(split_words(&words, 0.0, &t), vec!["I'd rather be", "What is"]);

        let pieces = [(" What", 0.2), (" is", 0.4), (" fa", 1.0), ("ther", 1.1), (".", 1.2), ("\u{2581}I'd", 2.0)];
        assert_eq!(words_from_pieces(pieces.into_iter()), vec![
            ("What".to_string(), 0.2), ("is".to_string(), 0.4), ("father.".to_string(), 1.0), ("I'd".to_string(), 2.0)]);

        let merged = merge_empty_turns(&[(0, 0.0, 2.0), (1, 2.0, 3.0), (0, 3.0, 5.0)], vec!["a".into(), "".into(), "b".into()]);
        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].1, "a b");
        assert_eq!(merged[0].0, (0, 0.0, 5.0));
    }

    #[test]
    fn speech_secs_counts_frames_per_speaker() {
        let track = vec![Some(0), Some(0), None, Some(1)];
        let s = speech_secs(&track, 2);
        assert!((s[0] - 2.0 * FRAME_SECS).abs() < 1e-9 && (s[1] - FRAME_SECS).abs() < 1e-9);
    }

    #[test]
    fn fill_gaps_uses_nearest_line() {
        let mut labels = vec![None, Some(1), None, None, Some(0)];
        fill_gaps(&mut labels);
        assert_eq!(labels, vec![Some(1), Some(1), Some(1), Some(0), Some(0)]);
        let mut none = vec![None, None];
        fill_gaps(&mut none);
        assert_eq!(none, vec![Some(0), Some(0)]);
    }

    fn test_params() -> Params {
        let env = |k: &str| std::env::var(k).ok().and_then(|v| v.parse::<f64>().ok());
        let d = Params::default();
        Params {
            threshold: env("DIAR_THRESHOLD").map_or(d.threshold, |v| v as f32),
            min_embed_secs: env("DIAR_MIN_EMBED").unwrap_or(d.min_embed_secs),
            min_turn_secs: env("DIAR_MIN_TURN").unwrap_or(d.min_turn_secs),
        }
    }

    /// End-to-end check of the embedding model. Needs DIARIZATION_TEST_DIR containing
    /// voxceleb_resnet34.onnx and v{0,1,2}_l{0,1,2}.wav (three voices x three lines, 16kHz).
    #[test]
    #[ignore]
    fn real_model_separates_three_voices() {
        let dir = PathBuf::from(std::env::var("DIARIZATION_TEST_DIR").expect("DIARIZATION_TEST_DIR"));
        let mut embedder = Embedder::load(&dir.join(EMBEDDING_MODEL.file)).unwrap();
        let order: Vec<(usize, usize)> = (0..3).flat_map(|l| (0..3).map(move |v| (v, l))).collect();
        let embs: Vec<Vec<f32>> = order
            .iter()
            .map(|(v, l)| {
                let audio = crate::audio::decoder::decode_audio_file(&dir.join(format!("v{v}_l{l}.wav"))).unwrap();
                embedder.embed(&audio.to_whisper_format()).unwrap()
            })
            .collect();
        // Synthetic voices from one TTS engine are unusually close (two male voices ~0.48), so
        // this checks the model can separate them at all, not the production threshold.
        assert_eq!(cluster(&embs, 0.5), vec![0, 1, 2, 0, 1, 2, 0, 1, 2]);
    }

    /// Any audio file, no transcript needed: diarize, transcribe with Parakeet (word
    /// timestamps), and print who said what. Needs DIARIZATION_TEST_DIR (both models),
    /// DIARIZATION_AUDIO (file) and PARAKEET_DIR (a Parakeet int8 model folder).
    #[test]
    #[ignore]
    fn real_audio_file() {
        let dir = PathBuf::from(std::env::var("DIARIZATION_TEST_DIR").unwrap());
        let file = PathBuf::from(std::env::var("DIARIZATION_AUDIO").unwrap());
        let audio = crate::audio::decoder::decode_audio_file(&file).unwrap().to_whisper_format();
        let mut seg = Segmenter::load(&dir.join(SEGMENTATION_MODEL.file)).unwrap();
        let mut emb = Embedder::load(&dir.join(EMBEDDING_MODEL.file)).unwrap();
        let params = test_params();
        let track = diarize(&mut seg, &mut emb, &audio, &params, |_| {}).unwrap().track;

        // Raw frame track as a compact timeline (who is active, 0.1 s resolution).
        let per = (0.1 / FRAME_SECS) as usize;
        let timeline: String = track.chunks(per).map(|c| {
            let mut counts = [0usize; 10];
            c.iter().flatten().for_each(|&s| counts[s.min(9)] += 1);
            let best = (0..10).max_by_key(|&i| counts[i]).unwrap();
            if counts[best] * 2 >= c.len() { char::from(b'1' + best as u8) } else { '.' }
        }).collect();
        println!("timeline (0.1 s per char, digit = speaker, . = silence):\n{timeline}");

        let mut asr = crate::parakeet_engine::ParakeetModel::new(std::env::var("PARAKEET_DIR").unwrap(), true).unwrap();
        let words = words_from_tokens(&asr.transcribe_samples(audio.clone()).unwrap());
        let turns = line_turns(&track, (0.0, audio.len() as f64 / SAMPLE_RATE), params.min_turn_secs);
        for ((spk, a, b), text) in turns.iter().zip(split_words(&words, 0.0, &turns)) {
            println!("[{a:5.1}s - {b:5.1}s] Speaker {}: {text}", spk + 1);
        }
        println!("words with times: {}", words.iter().map(|(w, t)| format!("{w}@{t:.1}")).collect::<Vec<_>>().join(" "));
    }

    /// Full pipeline on a recorded meeting folder (audio.mp4 + transcripts.json), printing
    /// each line's turns. Needs DIARIZATION_TEST_DIR (both models) and DIARIZATION_MEETING_DIR.
    #[test]
    #[ignore]
    fn real_meeting_folder() {
        let dir = PathBuf::from(std::env::var("DIARIZATION_TEST_DIR").unwrap());
        let folder = PathBuf::from(std::env::var("DIARIZATION_MEETING_DIR").unwrap());
        let json: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(folder.join("transcripts.json")).unwrap()).unwrap();
        let audio_path = crate::audio::retranscription::find_audio_file(&folder).unwrap();
        let audio = crate::audio::decoder::decode_audio_file(&audio_path).unwrap().to_whisper_format();
        let mut seg = Segmenter::load(&dir.join(SEGMENTATION_MODEL.file)).unwrap();
        let mut emb = Embedder::load(&dir.join(EMBEDDING_MODEL.file)).unwrap();
        let params = test_params();
        let started = std::time::Instant::now();
        let extracted = extract(&mut seg, &mut emb, &audio, &params, |_| {}).unwrap();
        println!("extracted {:.0}s of audio in {:.1}s ({} embeddings)", audio.len() as f64 / SAMPLE_RATE, started.elapsed().as_secs_f64(), extracted.embeddings.len());
        let thresholds: Vec<f32> = std::env::var("DIAR_THRESHOLDS").ok()
            .map(|v| v.split(',').filter_map(|t| t.parse().ok()).collect())
            .unwrap_or_else(|| vec![params.threshold]);
        let truth: Option<Vec<serde_json::Value>> = std::fs::read_to_string(folder.join("truth.json")).ok()
            .map(|t| serde_json::from_str(&t).unwrap());
        let mut track = Vec::new();
        for &th in &thresholds {
            track = extracted.resolve(th).track;
            let found: std::collections::BTreeSet<usize> = track.iter().flatten().copied().collect();
            print!("threshold {th:.2}: speakers found {}", found.len());
            // Frame accuracy with a many-to-one mapping of predicted speakers to true ones.
            if let Some(truth) = &truth {
                let mut pairs = Vec::new(); // (predicted, true) per speech frame
                for turn in truth {
                    let (a, b) = (turn["start"].as_f64().unwrap(), turn["end"].as_f64().unwrap());
                    let spk = turn["speaker"].as_u64().unwrap() as usize;
                    for f in (a / FRAME_SECS) as usize..((b / FRAME_SECS) as usize).min(track.len()) {
                        pairs.push((track[f], spk));
                    }
                }
                let n_true = pairs.iter().map(|p| p.1).max().map_or(0, |m| m + 1);
                let mut map = std::collections::HashMap::new();
                for &p in &found {
                    let best = (0..n_true).max_by_key(|&t| pairs.iter().filter(|x| x.0 == Some(p) && x.1 == t).count()).unwrap();
                    map.insert(p, best);
                }
                let correct = pairs.iter().filter(|(p, t)| p.and_then(|p| map.get(&p)) == Some(t)).count();
                print!(" | ACCURACY {:.1}% (true speakers {n_true})", 100.0 * correct as f64 / pairs.len() as f64);
            }
            println!();
        }
        if thresholds.len() > 1 {
            return;
        }
        for s in json["segments"].as_array().unwrap() {
            let (a, b) = (s["audio_start_time"].as_f64().unwrap(), s["audio_end_time"].as_f64().unwrap());
            let text = s["text"].as_str().unwrap();
            let t = line_turns(&track, (a, b), params.min_turn_secs);
            let texts = split_words(&words_evenly(text, b - a), a, &t);
            for ((spk, x, y), txt) in t.iter().zip(texts) {
                println!("[{x:6.1}-{y:6.1}] Speaker {}: {}", spk + 1, txt.chars().take(80).collect::<String>());
            }
            if t.is_empty() {
                println!("[{a:6.1}-{b:6.1}] (no speech detected): {}", text.chars().take(80).collect::<String>());
            }
        }
    }

    /// Voice memory across recordings, label-free: diarize the whole file as reference, enrol
    /// voices from the first half, match the second half. A match is right when both halves'
    /// speakers map to the same reference speaker. Needs DIARIZATION_TEST_DIR, DIARIZATION_AUDIO.
    #[test]
    #[ignore]
    fn real_voice_matching() {
        use crate::voices::{match_voices, MIN_SAMPLE_SECS};
        let dir = PathBuf::from(std::env::var("DIARIZATION_TEST_DIR").unwrap());
        let file = PathBuf::from(std::env::var("DIARIZATION_AUDIO").unwrap());
        let audio = crate::audio::decoder::decode_audio_file(&file).unwrap().to_whisper_format();
        let mut seg = Segmenter::load(&dir.join(SEGMENTATION_MODEL.file)).unwrap();
        let mut emb = Embedder::load(&dir.join(EMBEDDING_MODEL.file)).unwrap();
        let params = test_params();
        let full = diarize(&mut seg, &mut emb, &audio, &params, |_| {}).unwrap();
        let mid = audio.len() / 2 / FRAME_STEP_SAMPLES * FRAME_STEP_SAMPLES;
        let a = diarize(&mut seg, &mut emb, &audio[..mid], &params, |_| {}).unwrap();
        let b = diarize(&mut seg, &mut emb, &audio[mid..], &params, |_| {}).unwrap();

        // Reference speaker most often under each half-speaker's frames.
        let majority = |d: &Diarization, offset: usize| -> Vec<Option<usize>> {
            (0..d.centroids.len())
                .map(|id| {
                    let mut counts = std::collections::HashMap::new();
                    for (f, s) in d.track.iter().enumerate() {
                        if *s == Some(id) {
                            if let Some(Some(r)) = full.track.get(offset + f) {
                                *counts.entry(*r).or_insert(0usize) += 1;
                            }
                        }
                    }
                    counts.into_iter().max_by_key(|(_, n)| *n).map(|(r, _)| r)
                })
                .collect()
        };
        let (ra, rb) = (majority(&a, 0), majority(&b, mid / FRAME_STEP_SAMPLES));
        let secs_a = speech_secs(&a.track, a.centroids.len());
        let profiles: Vec<(String, Vec<f32>)> = a.centroids.iter().enumerate()
            .filter(|(i, c)| secs_a[*i] >= MIN_SAMPLE_SECS && !c.is_empty())
            .map(|(i, c)| (format!("ref{}", ra[i].map_or(99, |r| r)), c.clone()))
            .collect();
        println!("reference speakers: {}, enrolled: {}", full.centroids.len(), profiles.len());
        for t in [0.40f32, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70] {
            let mut names = vec![None; b.centroids.len()];
            match_voices(&b.centroids, &mut names, &profiles, t);
            let (mut right, mut wrong, mut unnamed) = (0, 0, 0);
            for (i, n) in names.iter().enumerate() {
                match n {
                    Some(n) if rb[i].map(|r| format!("ref{r}")).as_deref() == Some(n.as_str()) => right += 1,
                    Some(_) => wrong += 1,
                    None => unnamed += 1,
                }
            }
            println!("threshold {t:.2}: right {right}, wrong {wrong}, unnamed {unnamed}");
        }
    }
}
