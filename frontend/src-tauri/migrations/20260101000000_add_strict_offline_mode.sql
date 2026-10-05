-- Strict Offline Mode: when enabled (1), the app refuses to contact cloud LLM
-- providers and the auto-updater, and requires the Ollama/custom-OpenAI endpoint to
-- resolve to a loopback address. See security/reports/03-offline-architecture.md and
-- security/RESIDUAL_RISKS.md #3 — previously no such flag existed anywhere.
ALTER TABLE settings ADD COLUMN strictOfflineMode INTEGER NOT NULL DEFAULT 0;
