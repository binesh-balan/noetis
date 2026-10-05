//! NVIDIA Nemotron 3 Diarization (streaming Sortformer, 100M params, OpenMDW-1.1) on ONNX
//! Runtime: one model predicts, every 10 ms, the activity of up to 8 speakers ordered by first
//! arrival — no separate embedding/clustering stage to tune.
//!
//! The ONNX export (onnx-community, int8) runs one step: `[speaker cache | FIFO | chunk]`.
//! This module ports the rest of Transformers' offline `Nemotron3DiarizationForAudioFrameClassification`
//! forward: the log-mel front end (`NemotronAsrStreamingFeatureExtractor`), the chunk loop, and
//! the Arrival-Order Speaker Cache (`Nemotron3DiarizationSpeakerCache.update/_compress`).
//! Parity with the reference is checked by the `parity_*` tests.

use anyhow::{anyhow, Result};
use ndarray::{Array2, Array3};
use ort::execution_providers::CPUExecutionProvider;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;
use std::path::Path;

/// Output frame length: one prediction every 10 ms.
pub const FRAME_SECS: f64 = 0.01;

// Feature extractor (preprocessor_config.json).
const N_MELS: usize = 128;
const N_FFT: usize = 512;
const HOP: usize = 160;
const WIN: usize = 400;
const PREEMPHASIS: f32 = 0.97;
const LOG_GUARD: f32 = 5.960_464_5e-8; // 2^-24

// Model / offline chunking (config.json).
const HIDDEN: usize = 512;
const SPEAKERS: usize = 8;
const SUBSAMPLING: usize = 8;
const CHUNK: usize = 340;
const RIGHT_CONTEXT: usize = 40;
const FIFO_LEN: usize = 40;
const UPDATE_PERIOD: usize = 300;
// Speaker cache policy (streaming_config).
const CACHE_LEN: usize = 264;
const SILENCE_FRAMES: usize = 1;
const SCORE_THRESHOLD: f32 = 0.25;
const LATEST_BOOST: f32 = 0.05;
const BUDGET: usize = CACHE_LEN / SPEAKERS - SILENCE_FRAMES;
const MIN_POSITIVE: usize = BUDGET / 2; // floor(budget * 0.5)
const STRONG_BOOSTED: usize = BUDGET * 3 / 4; // floor(budget * 0.75)
const WEAK_BOOSTED: usize = BUDGET * 3 / 2; // floor(budget * 1.5)

// ---------------------------------------------------------------------------
// Front end: pre-emphasis, centered STFT (zero padding, symmetric Hann 400 in a 512 FFT),
// power spectrum, 128 Slaney mel filters (librosa), natural log with a 2^-24 guard.
// ---------------------------------------------------------------------------

fn hz_to_mel(f: f64) -> f64 {
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let logstep = 6.4f64.ln() / 27.0;
    if f >= min_log_hz { min_log_hz / f_sp + (f / min_log_hz).ln() / logstep } else { f / f_sp }
}

fn mel_to_hz(m: f64) -> f64 {
    let (f_sp, min_log_hz) = (200.0 / 3.0, 1000.0);
    let (min_log_mel, logstep) = (min_log_hz / f_sp, 6.4f64.ln() / 27.0);
    if m >= min_log_mel { min_log_hz * (logstep * (m - min_log_mel)).exp() } else { f_sp * m }
}

/// librosa.filters.mel(sr=16000, n_fft=512, n_mels=128, fmin=0, fmax=8000, norm="slaney").
fn mel_filters() -> Vec<Vec<f32>> {
    let bins = N_FFT / 2 + 1;
    let fft_freqs: Vec<f64> = (0..bins).map(|k| k as f64 * 8000.0 / (bins - 1) as f64).collect();
    let (lo, hi) = (hz_to_mel(0.0), hz_to_mel(8000.0));
    let mel_f: Vec<f64> = (0..N_MELS + 2).map(|i| mel_to_hz(lo + (hi - lo) * i as f64 / (N_MELS + 1) as f64)).collect();
    (0..N_MELS)
        .map(|i| {
            let enorm = 2.0 / (mel_f[i + 2] - mel_f[i]);
            fft_freqs
                .iter()
                .map(|&f| {
                    let lower = (f - mel_f[i]) / (mel_f[i + 1] - mel_f[i]);
                    let upper = (mel_f[i + 2] - f) / (mel_f[i + 2] - mel_f[i + 1]);
                    (lower.min(upper).max(0.0) * enorm) as f32
                })
                .collect()
        })
        .collect()
}

/// Log-mel features (frames x 128, row-major) and the number of valid frames. Frames past the
/// valid count are zero, as in the reference.
pub fn features(samples: &[f32]) -> (Vec<f32>, usize) {
    let len = samples.len();
    let mut x = Vec::with_capacity(len);
    for i in 0..len {
        x.push(if i == 0 { samples[0] } else { samples[i] - PREEMPHASIS * samples[i - 1] });
    }
    // center=True: pad n_fft/2 zeros on both sides.
    let mut padded = vec![0f32; len + N_FFT];
    padded[N_FFT / 2..N_FFT / 2 + len].copy_from_slice(&x);
    let frames = 1 + len / HOP;
    let valid = len / HOP;

    // Symmetric Hann(400) centered inside the 512-sample frame (torch pads the window).
    let offset = (N_FFT - WIN) / 2;
    let mut window = vec![0f32; N_FFT];
    for n in 0..WIN {
        window[offset + n] = (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / (WIN - 1) as f64).cos()) as f32;
    }
    let filters = mel_filters();
    let fft = realfft::RealFftPlanner::<f32>::new().plan_fft_forward(N_FFT);
    let mut buf = vec![0f32; N_FFT];
    let mut spectrum = fft.make_output_vec();
    let mut power = vec![0f32; N_FFT / 2 + 1];
    let mut out = vec![0f32; frames * N_MELS];
    for t in 0..valid {
        let start = t * HOP;
        for (b, (s, w)) in buf.iter_mut().zip(padded[start..start + N_FFT].iter().zip(&window)) {
            *b = s * w;
        }
        fft.process(&mut buf, &mut spectrum).expect("fixed-size FFT");
        for (p, c) in power.iter_mut().zip(&spectrum) {
            *p = c.norm_sqr();
        }
        let row = &mut out[t * N_MELS..(t + 1) * N_MELS];
        for (r, f) in row.iter_mut().zip(&filters) {
            let e: f32 = f.iter().zip(&power).map(|(w, p)| w * p).sum();
            *r = (e + LOG_GUARD).ln();
        }
    }
    (out, valid)
}

// ---------------------------------------------------------------------------
// Arrival-Order Speaker Cache + FIFO (offline sizes).
// ---------------------------------------------------------------------------

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

#[derive(Default)]
struct SpeakerCache {
    embeds: Vec<f32>, // rows of HIDDEN
    probs: Vec<f32>,  // rows of SPEAKERS
    fifo: Vec<f32>,   // rows of HIDDEN
    compressed: bool,
}

impl SpeakerCache {
    fn rows(v: &[f32], width: usize) -> usize {
        v.len() / width
    }

    fn cached(&self) -> Vec<f32> {
        [self.embeds.as_slice(), self.fifo.as_slice()].concat()
    }

    /// `input_embeds` = [cache | fifo | chunk (+look-ahead)], `logits` at the 10 ms rate over them.
    fn update(&mut self, input_embeds: &[f32], logits: &[f32], silence: &[f32], n_chunk: usize, mask: &[f32]) {
        let nc = Self::rows(&self.embeds, HIDDEN);
        let nf = Self::rows(&self.fifo, HIDDEN);
        // Speaker probabilities at the encoder rate: mean of 8 sigmoid frames, zeroed on padding.
        let t_frames = logits.len() / (SPEAKERS * SUBSAMPLING);
        let mut probs = vec![0f32; t_frames * SPEAKERS];
        for t in 0..t_frames {
            for s in 0..SPEAKERS {
                let sum: f32 = (0..SUBSAMPLING).map(|k| sigmoid(logits[((t * SUBSAMPLING + k) * SPEAKERS) + s])).sum();
                probs[t * SPEAKERS + s] = sum / SUBSAMPLING as f32 * mask.get(t).copied().unwrap_or(0.0);
            }
        }
        let chunk = &input_embeds[(nc + nf) * HIDDEN..(nc + nf + n_chunk) * HIDDEN];
        let mut fifo = [self.fifo.as_slice(), chunk].concat();
        let fifo_rows = Self::rows(&fifo, HIDDEN);
        let popped = if fifo_rows <= FIFO_LEN { 0 } else { UPDATE_PERIOD.max(fifo_rows - FIFO_LEN).min(fifo_rows) };
        if popped > 0 {
            let fifo_probs = &probs[nc * SPEAKERS..(nc + fifo_rows) * SPEAKERS];
            let stored: Vec<f32> = if self.compressed { self.probs.clone() } else { probs[..nc * SPEAKERS].to_vec() };
            let mut cache_embeds = [self.embeds.as_slice(), &fifo[..popped * HIDDEN]].concat();
            let mut cache_probs = [stored.as_slice(), &fifo_probs[..popped * SPEAKERS]].concat();
            fifo.drain(..popped * HIDDEN);
            if Self::rows(&cache_embeds, HIDDEN) > CACHE_LEN {
                (cache_embeds, cache_probs) = compress(&cache_embeds, &cache_probs, silence);
                self.compressed = true;
            }
            self.embeds = cache_embeds;
            self.probs = cache_probs;
        }
        self.fifo = fifo;
    }
}

/// `_get_frame_scores`: per (frame, speaker) importance; -inf for frames not worth keeping.
fn frame_scores(probs: &[f32], n: usize) -> Vec<f32> {
    let (th_ln, half_ln) = (SCORE_THRESHOLD.ln(), 0.5f32.ln());
    let mut scores = vec![0f32; n * SPEAKERS];
    for f in 0..n {
        let p = &probs[f * SPEAKERS..(f + 1) * SPEAKERS];
        let lc: Vec<f32> = p.iter().map(|&x| (1.0 - x).max(SCORE_THRESHOLD).ln()).collect();
        let lc_sum: f32 = lc.iter().sum();
        for s in 0..SPEAKERS {
            let lp = if p[s] >= SCORE_THRESHOLD { p[s].ln() } else { th_ln };
            scores[f * SPEAKERS + s] = if p[s] > 0.5 { lp - lc[s] + lc_sum - half_ln } else { f32::NEG_INFINITY };
        }
    }
    for s in 0..SPEAKERS {
        let positives = (0..n).filter(|&f| scores[f * SPEAKERS + s] > 0.0).count();
        if positives >= MIN_POSITIVE {
            for f in 0..n {
                let (v, speech) = (scores[f * SPEAKERS + s], probs[f * SPEAKERS + s] > 0.5);
                if speech && v <= 0.0 {
                    scores[f * SPEAKERS + s] = f32::NEG_INFINITY;
                }
            }
        }
    }
    scores
}

/// Indices of the `k` largest values, ties to the lower index (torch.topk on CPU).
fn top_k(values: &[f32], k: usize) -> Vec<usize> {
    let mut idx: Vec<usize> = (0..values.len()).collect();
    idx.sort_by(|&a, &b| values[b].total_cmp(&values[a]).then(a.cmp(&b)));
    idx.truncate(k);
    idx
}

fn boost(scores: &mut [f32], n: usize, k: usize, amount: f32) {
    for s in 0..SPEAKERS {
        let column: Vec<f32> = (0..n).map(|f| scores[f * SPEAKERS + s]).collect();
        for f in top_k(&column, k) {
            scores[f * SPEAKERS + s] += amount;
        }
    }
}

/// `_compress`: keep the CACHE_LEN most useful frames, grouped by speaker in original order,
/// with SILENCE_FRAMES slots per speaker filled by the learned silence embedding.
fn compress(embeds: &[f32], probs: &[f32], silence: &[f32]) -> (Vec<f32>, Vec<f32>) {
    let n = probs.len() / SPEAKERS;
    let mut scores = frame_scores(probs, n);
    for f in CACHE_LEN..n {
        for s in 0..SPEAKERS {
            scores[f * SPEAKERS + s] += LATEST_BOOST;
        }
    }
    let half_ln = 0.5f32.ln();
    boost(&mut scores, n, STRONG_BOOSTED, -2.0 * half_ln);
    boost(&mut scores, n, WEAK_BOOSTED, -half_ln);
    let ns = n + SILENCE_FRAMES;
    // Speaker-major flattening, silence rows scored +inf.
    let flat: Vec<f32> = (0..SPEAKERS)
        .flat_map(|s| (0..ns).map(move |f| (s, f)))
        .map(|(s, f)| if f < n { scores[f * SPEAKERS + s] } else { f32::INFINITY })
        .collect();
    let sentinel = ns * SPEAKERS;
    let mut chosen: Vec<usize> = top_k(&flat, CACHE_LEN)
        .into_iter()
        .map(|i| if flat[i] == f32::NEG_INFINITY { sentinel } else { i })
        .collect();
    chosen.sort_unstable();
    let (mut out_e, mut out_p) = (Vec::with_capacity(CACHE_LEN * HIDDEN), Vec::with_capacity(CACHE_LEN * SPEAKERS));
    for i in chosen {
        let frame = if i == sentinel { n } else { (i % ns).min(n) };
        if frame == n {
            out_e.extend_from_slice(silence);
            out_p.extend(std::iter::repeat(0.0).take(SPEAKERS));
        } else {
            out_e.extend_from_slice(&embeds[frame * HIDDEN..(frame + 1) * HIDDEN]);
            out_p.extend_from_slice(&probs[frame * SPEAKERS..(frame + 1) * SPEAKERS]);
        }
    }
    (out_e, out_p)
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

pub struct Diarizer {
    session: Session,
}

impl Diarizer {
    /// `model` is the `.onnx` file; its `.onnx_data` weights must sit next to it.
    pub fn load(model: &Path) -> Result<Self> {
        crate::ensure_onnx_runtime_available()?;
        let session = Session::builder()?
            .with_optimization_level(GraphOptimizationLevel::Level3)?
            .with_execution_providers(vec![CPUExecutionProvider::default().build()])?
            .commit_from_file(model)?;
        Ok(Self { session })
    }

    /// Speaker logits (frames x 8, row-major) for log-mel `feats` with `valid` real frames.
    pub fn logits(&mut self, feats: &[f32], valid: usize, mut progress: impl FnMut(f64)) -> Result<Vec<f32>> {
        let frames = feats.len() / N_MELS;
        let pad = (SUBSAMPLING - frames % SUBSAMPLING) % SUBSAMPLING;
        let enc = (frames + pad) / SUBSAMPLING;
        let mut padded = feats.to_vec();
        padded.resize((frames + pad) * N_MELS, 0.0);
        // Encoder-rate validity: every 8th mel frame.
        let enc_mask: Vec<f32> = (0..enc).map(|e| if e * SUBSAMPLING < valid { 1.0 } else { 0.0 }).collect();

        let mut cache = SpeakerCache::default();
        let mut out = Vec::with_capacity(frames * SPEAKERS);
        let mut start = 0;
        while start < enc {
            let end = (start + CHUNK).min(enc);
            let stop = (end + RIGHT_CONTEXT).min(enc);
            let n_chunk = end - start;
            let cached = cache.cached();
            let n_cached = cached.len() / HIDDEN;
            let mask: Vec<f32> = std::iter::repeat(1.0).take(n_cached).chain(enc_mask[start..stop].iter().copied()).collect();

            let input = Array3::from_shape_vec((1, (stop - start) * SUBSAMPLING, N_MELS), padded[start * SUBSAMPLING * N_MELS..stop * SUBSAMPLING * N_MELS].to_vec())?;
            let cached_arr = Array3::from_shape_vec((1, n_cached, HIDDEN), cached.clone())?;
            let mask_arr = Array2::from_shape_vec((1, mask.len()), mask.iter().map(|&m| m as i64).collect())?;
            let outputs = self.session.run(ort::inputs![
                "input_features" => TensorRef::from_array_view(input.view())?,
                "cached_embeds" => TensorRef::from_array_view(cached_arr.view())?,
                "attention_mask" => TensorRef::from_array_view(mask_arr.view())?,
            ])?;
            let get = |name: &str| -> Result<Vec<f32>> {
                Ok(outputs.get(name).ok_or_else(|| anyhow!("missing output {name}"))?.try_extract_array::<f32>()?.iter().copied().collect())
            };
            let (logits, chunk_embeds, silence) = (get("logits")?, get("chunk_embeds")?, get("silence_embeds")?);

            let input_embeds = [cached.as_slice(), chunk_embeds.as_slice()].concat();
            cache.update(&input_embeds, &logits, &silence, n_chunk, &mask);
            out.extend_from_slice(&logits[n_cached * SUBSAMPLING * SPEAKERS..(n_cached + n_chunk) * SUBSAMPLING * SPEAKERS]);
            start = end;
            progress(start as f64 / enc as f64);
        }
        out.truncate(frames * SPEAKERS);
        Ok(out)
    }
}

/// Dominant speaker per 10 ms frame (probability >= 0.5), None for silence. Speaker ids are
/// in order of first arrival, as the model emits them.
pub fn speaker_track(logits: &[f32]) -> Vec<Option<usize>> {
    logits
        .chunks(SPEAKERS)
        .map(|row| {
            let (best, &v) = row.iter().enumerate().max_by(|a, b| a.1.total_cmp(b.1)).unwrap();
            (sigmoid(v) >= 0.5).then_some(best)
        })
        .collect()
}

/// Full pipeline: 16 kHz mono samples -> speaker per 10 ms frame.
pub fn diarize(diarizer: &mut Diarizer, samples: &[f32], progress: impl FnMut(f64)) -> Result<Vec<Option<usize>>> {
    let (feats, valid) = features(samples);
    Ok(speaker_track(&diarizer.logits(&feats, valid, progress)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn read_f32(path: &Path) -> Vec<f32> {
        std::fs::read(path).unwrap().chunks(4).map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]])).collect()
    }

    #[test]
    fn slaney_mel_matches_librosa_spot_values() {
        // librosa.filters.mel(sr=16000, n_fft=512, n_mels=128, norm="slaney")
        let f = mel_filters();
        assert_eq!((f.len(), f[0].len()), (128, 257));
        assert!((hz_to_mel(1000.0) - 15.0).abs() < 1e-9 && (mel_to_hz(hz_to_mel(3210.0)) - 3210.0).abs() < 1e-6);
        // Every filter is a non-negative triangle with some weight.
        assert!(f.iter().all(|row| row.iter().all(|&w| w >= 0.0) && row.iter().any(|&w| w > 0.0)));
    }

    #[test]
    fn top_k_breaks_ties_to_lower_index() {
        assert_eq!(top_k(&[1.0, 3.0, 3.0, 2.0], 2), vec![1, 2]);
        assert_eq!(top_k(&[f32::NEG_INFINITY, 0.0, f32::INFINITY], 3), vec![2, 1, 0]);
    }

    /// Needs NEMOTRON_REF (dumps from the validated Python reference): features must match.
    #[test]
    #[ignore]
    fn parity_features() {
        let dir = PathBuf::from(std::env::var("NEMOTRON_REF").unwrap());
        for name in ["clip", "meeting"] {
            let pcm = read_f32(&dir.join(format!("{name}.pcm")));
            let reference = read_f32(&dir.join(format!("{name}.features.f32")));
            let (ours, valid) = features(&pcm);
            assert_eq!(ours.len(), reference.len(), "{name}: frame count");
            let max = ours.iter().zip(&reference).map(|(a, b)| (a - b).abs()).fold(0f32, f32::max);
            println!("{name}: {} frames ({valid} valid), max |feature diff| {max:.2e}", ours.len() / N_MELS);
            assert!(max < 1e-2, "{name}: features differ by {max}");
        }
    }

    /// Needs NEMOTRON_REF and NEMOTRON_MODEL (model_quantized.onnx): speaker decisions must match.
    #[test]
    #[ignore]
    fn parity_logits() {
        let dir = PathBuf::from(std::env::var("NEMOTRON_REF").unwrap());
        let mut d = Diarizer::load(Path::new(&std::env::var("NEMOTRON_MODEL").unwrap())).unwrap();
        let cases = std::env::var("NEMOTRON_CASES").unwrap_or_else(|_| "clip,meeting,convo,ami".into());
        for name in cases.split(',') {
            let pcm = read_f32(&dir.join(format!("{name}.pcm")));
            let reference = read_f32(&dir.join(format!("{name}.logits.f32")));
            let started = std::time::Instant::now();
            let (feats, valid) = features(&pcm);
            let ours = d.logits(&feats, valid, |_| {}).unwrap();
            let secs = started.elapsed().as_secs_f64();
            assert_eq!(ours.len(), reference.len(), "{name}: frame count");
            let (a, b) = (speaker_track(&ours), speaker_track(&reference));
            let same = a.iter().zip(&b).filter(|(x, y)| x == y).count() as f64 / a.len() as f64 * 100.0;
            let max = ours.iter().zip(&reference).map(|(x, y)| (x - y).abs()).fold(0f32, f32::max);
            println!("{name}: {:.0}s audio in {secs:.1}s | identical speaker decisions {same:.2}% | max |logit diff| {max:.3}",
                pcm.len() as f64 / 16000.0);
            assert!(same > 99.0, "{name}: only {same:.2}% of frames agree");
        }
    }
}
