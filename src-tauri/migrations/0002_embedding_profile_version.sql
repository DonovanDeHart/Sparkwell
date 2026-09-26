-- Record which retrieval-profile recipe produced each cached vector, next to
-- the model and dimensions, so vectors from another model or recipe are never
-- mixed with current ones. Existing rows (older recipes) are re-embedded.
ALTER TABLE embeddings ADD COLUMN profile_version TEXT NOT NULL DEFAULT '';
