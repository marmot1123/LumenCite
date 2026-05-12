-- LLM 要約用カラムを entries に追加 (v0.1.0)
ALTER TABLE entries ADD COLUMN summary TEXT;
ALTER TABLE entries ADD COLUMN summary_model TEXT;
ALTER TABLE entries ADD COLUMN summary_generated_at TEXT;
