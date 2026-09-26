//! Voice memory: speaker embeddings kept per meeting (`voice_samples`), so later meetings
//! can name speakers. A known voice is any label that isn't "Speaker N". Local only.

use serde::Serialize;
use sqlx::{Row, Sqlite, SqlitePool, Transaction};
use std::collections::BTreeMap;
use tauri::{AppHandle, Manager, Runtime};

use crate::state::AppState;

pub fn is_placeholder(label: &str) -> bool {
    label.strip_prefix("Speaker ").is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

fn from_blob(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4).map(|c| f32::from_le_bytes([c[0], c[1], c[2], c[3]])).collect()
}

pub fn normalize(mut v: Vec<f32>) -> Vec<f32> {
    let n = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 0.0 {
        v.iter_mut().for_each(|x| *x /= n);
    }
    v
}

pub struct Sample {
    pub label: String,
    pub embedding: Vec<f32>,
    pub speech_secs: f64,
}

/// Replaces a meeting's samples.
pub async fn save_meeting_samples(pool: &SqlitePool, meeting_id: &str, samples: &[Sample]) -> sqlx::Result<()> {
    let now = chrono::Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    sqlx::query("DELETE FROM voice_samples WHERE meeting_id = ?").bind(meeting_id).execute(&mut *tx).await?;
    for s in samples {
        sqlx::query("INSERT INTO voice_samples (meeting_id, label, embedding, speech_secs, created_at) VALUES (?, ?, ?, ?, ?)")
            .bind(meeting_id).bind(&s.label).bind(to_blob(&s.embedding)).bind(s.speech_secs).bind(&now)
            .execute(&mut *tx).await?;
    }
    tx.commit().await
}

/// Known voices as (name, mean embedding, normalised), ignoring one meeting's samples.
pub async fn load_profiles(pool: &SqlitePool, exclude_meeting: &str) -> sqlx::Result<Vec<(String, Vec<f32>)>> {
    let rows = sqlx::query("SELECT label, embedding FROM voice_samples WHERE meeting_id != ?")
        .bind(exclude_meeting)
        .fetch_all(pool)
        .await?;
    let mut sums: BTreeMap<String, Vec<f32>> = BTreeMap::new();
    for r in rows {
        let label: String = r.get("label");
        if is_placeholder(&label) {
            continue;
        }
        let e = from_blob(r.get::<Vec<u8>, _>("embedding").as_slice());
        let sum = sums.entry(label).or_insert_with(|| vec![0.0; e.len()]);
        sum.iter_mut().zip(&e).for_each(|(a, b)| *a += b);
    }
    Ok(sums.into_iter().map(|(k, v)| (k, normalize(v))).collect())
}

async fn sample_row(tx: &mut Transaction<'_, Sqlite>, meeting_id: &str, label: &str) -> sqlx::Result<Option<(Vec<f32>, f64)>> {
    let row = sqlx::query("SELECT embedding, speech_secs FROM voice_samples WHERE meeting_id = ? AND label = ?")
        .bind(meeting_id).bind(label).fetch_optional(&mut **tx).await?;
    Ok(row.map(|r| (from_blob(r.get::<Vec<u8>, _>("embedding").as_slice()), r.get("speech_secs"))))
}

/// Moves a meeting's sample from `from` to `to`; merges (speech-weighted) if `to` exists.
pub async fn relabel_in_meeting(tx: &mut Transaction<'_, Sqlite>, meeting_id: &str, from: &str, to: &str) -> sqlx::Result<()> {
    if from == to {
        return Ok(());
    }
    let Some((a, sa)) = sample_row(tx, meeting_id, from).await? else { return Ok(()) };
    match sample_row(tx, meeting_id, to).await? {
        None => {
            sqlx::query("UPDATE voice_samples SET label = ? WHERE meeting_id = ? AND label = ?")
                .bind(to).bind(meeting_id).bind(from).execute(&mut **tx).await?;
        }
        Some((b, sb)) => {
            let merged = normalize(a.iter().zip(&b).map(|(x, y)| x * sa as f32 + y * sb as f32).collect());
            sqlx::query("UPDATE voice_samples SET embedding = ?, speech_secs = ? WHERE meeting_id = ? AND label = ?")
                .bind(to_blob(&merged)).bind(sa + sb).bind(meeting_id).bind(to).execute(&mut **tx).await?;
            sqlx::query("DELETE FROM voice_samples WHERE meeting_id = ? AND label = ?")
                .bind(meeting_id).bind(from).execute(&mut **tx).await?;
        }
    }
    Ok(())
}

#[derive(Debug, Serialize, PartialEq)]
pub struct VoiceSummary {
    pub name: String,
    pub meetings: i64,
    pub last_heard: String,
}

pub async fn list_voices(pool: &SqlitePool) -> sqlx::Result<Vec<VoiceSummary>> {
    let rows = sqlx::query("SELECT label, COUNT(*) AS n, MAX(created_at) AS last FROM voice_samples GROUP BY label ORDER BY label COLLATE NOCASE")
        .fetch_all(pool).await?;
    Ok(rows.into_iter()
        .map(|r| VoiceSummary { name: r.get("label"), meetings: r.get("n"), last_heard: r.get("last") })
        .filter(|v| !is_placeholder(&v.name))
        .collect())
}

/// Renames a known voice everywhere: its samples (merging per meeting) and transcript lines.
pub async fn rename_voice(pool: &SqlitePool, from: &str, to: &str) -> sqlx::Result<()> {
    let mut tx = pool.begin().await?;
    let meetings: Vec<String> = sqlx::query_scalar("SELECT meeting_id FROM voice_samples WHERE label = ?")
        .bind(from).fetch_all(&mut *tx).await?;
    for m in meetings {
        relabel_in_meeting(&mut tx, &m, from, to).await?;
    }
    sqlx::query("UPDATE transcripts SET speaker = ? WHERE speaker = ?").bind(to).bind(from).execute(&mut *tx).await?;
    tx.commit().await
}

/// Deletes a voice's samples. Transcripts keep the name.
pub async fn forget_voice(pool: &SqlitePool, name: &str) -> sqlx::Result<u64> {
    Ok(sqlx::query("DELETE FROM voice_samples WHERE label = ?").bind(name).execute(pool).await?.rows_affected())
}

fn pool<R: Runtime>(app: &AppHandle<R>) -> Result<SqlitePool, String> {
    Ok(app.try_state::<AppState>().ok_or("App state not available")?.db_manager.pool().clone())
}

fn valid_name(name: &str) -> Result<&str, String> {
    let name = name.trim();
    if name.is_empty() || name.chars().count() > 64 {
        return Err("Speaker name must be 1-64 characters".into());
    }
    Ok(name)
}

#[tauri::command]
pub async fn api_list_voices<R: Runtime>(app: AppHandle<R>) -> Result<Vec<VoiceSummary>, String> {
    list_voices(&pool(&app)?).await.map_err(|e| format!("Failed to list voices: {e}"))
}

#[tauri::command]
pub async fn api_rename_voice<R: Runtime>(app: AppHandle<R>, from: String, to: String) -> Result<(), String> {
    let to = valid_name(&to)?;
    rename_voice(&pool(&app)?, &from, to).await.map_err(|e| format!("Failed to rename voice: {e}"))
}

#[tauri::command]
pub async fn api_forget_voice<R: Runtime>(app: AppHandle<R>, name: String) -> Result<u64, String> {
    forget_voice(&pool(&app)?, &name).await.map_err(|e| format!("Failed to forget voice: {e}"))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    pub(crate) async fn test_pool() -> SqlitePool {
        // One connection: every new connection to sqlite::memory: is a fresh database.
        let pool = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        for m in ["m1", "m2"] {
            sqlx::query("INSERT INTO meetings (id, title, created_at, updated_at) VALUES (?, ?, '2026-09-26', '2026-09-26')")
                .bind(m).bind(m).execute(&pool).await.unwrap();
        }
        pool
    }

    fn sample(label: &str, e: [f32; 2], secs: f64) -> Sample {
        Sample { label: label.into(), embedding: normalize(e.to_vec()), speech_secs: secs }
    }

    #[test]
    fn placeholder_labels() {
        assert!(is_placeholder("Speaker 3"));
        assert!(!is_placeholder("Speaker"));
        assert!(!is_placeholder("Speaker X"));
        assert!(!is_placeholder("Priya"));
    }

    #[tokio::test]
    async fn profiles_average_named_samples_and_skip_the_current_meeting() {
        let pool = test_pool().await;
        save_meeting_samples(&pool, "m1", &[sample("Priya", [1.0, 0.0], 30.0), sample("Speaker 2", [0.0, 1.0], 30.0)]).await.unwrap();
        save_meeting_samples(&pool, "m2", &[sample("Priya", [0.0, 1.0], 30.0)]).await.unwrap();
        let p = load_profiles(&pool, "none").await.unwrap();
        assert_eq!(p.len(), 1, "placeholder skipped");
        assert!((p[0].1[0] - p[0].1[1]).abs() < 1e-6, "mean of both samples");
        assert_eq!(load_profiles(&pool, "m2").await.unwrap()[0].1, vec![1.0, 0.0]);
    }

    #[tokio::test]
    async fn relabel_moves_or_merges() {
        let pool = test_pool().await;
        save_meeting_samples(&pool, "m1", &[sample("Speaker 1", [1.0, 0.0], 10.0), sample("Speaker 2", [0.0, 1.0], 30.0)]).await.unwrap();
        let mut tx = pool.begin().await.unwrap();
        relabel_in_meeting(&mut tx, "m1", "Speaker 1", "Dana").await.unwrap();
        relabel_in_meeting(&mut tx, "m1", "Speaker 2", "Dana").await.unwrap();
        tx.commit().await.unwrap();
        let v = list_voices(&pool).await.unwrap();
        assert_eq!(v.len(), 1);
        assert_eq!(v[0].name, "Dana");
        let dana = &load_profiles(&pool, "none").await.unwrap()[0].1;
        assert!(dana[1] > dana[0], "merge weighted toward the 30 s sample");
    }

    #[tokio::test]
    async fn rename_and_forget_voice() {
        let pool = test_pool().await;
        save_meeting_samples(&pool, "m1", &[sample("Pria", [1.0, 0.0], 30.0)]).await.unwrap();
        save_meeting_samples(&pool, "m2", &[sample("Pria", [1.0, 0.0], 30.0)]).await.unwrap();
        rename_voice(&pool, "Pria", "Priya").await.unwrap();
        let v = list_voices(&pool).await.unwrap();
        assert_eq!((v[0].name.as_str(), v[0].meetings), ("Priya", 2));
        assert_eq!(forget_voice(&pool, "Priya").await.unwrap(), 2);
        assert!(list_voices(&pool).await.unwrap().is_empty());
    }
}
