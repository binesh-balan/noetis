-- Voice memory: one speaker embedding per speaker per meeting, labelled with the speaker's
-- current name in that meeting. Labels other than "Speaker N" are known voices.
CREATE TABLE IF NOT EXISTS voice_samples (
    meeting_id  TEXT NOT NULL,
    label       TEXT NOT NULL,
    embedding   BLOB NOT NULL,  -- 256 x f32 little-endian, L2-normalised
    speech_secs REAL NOT NULL,
    created_at  TEXT NOT NULL,
    PRIMARY KEY (meeting_id, label),
    FOREIGN KEY (meeting_id) REFERENCES meetings(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_voice_samples_label ON voice_samples(label);
