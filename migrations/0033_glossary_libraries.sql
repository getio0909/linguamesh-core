CREATE TABLE IF NOT EXISTS glossaries (
    glossary_id TEXT PRIMARY KEY,
    created_at INTEGER NOT NULL DEFAULT (unixepoch()),
    updated_at INTEGER NOT NULL DEFAULT (unixepoch())
);

CREATE TABLE IF NOT EXISTS glossary_terms (
    glossary_id TEXT NOT NULL REFERENCES glossaries(glossary_id) ON DELETE CASCADE,
    term_index INTEGER NOT NULL,
    source_term TEXT NOT NULL,
    target_term TEXT NOT NULL,
    source_locale TEXT,
    target_locale TEXT,
    case_sensitive INTEGER NOT NULL,
    whole_word INTEGER NOT NULL,
    immutable INTEGER NOT NULL,
    domain TEXT,
    priority INTEGER NOT NULL,
    notes TEXT,
    enabled INTEGER NOT NULL,
    PRIMARY KEY (glossary_id, term_index)
);

CREATE INDEX IF NOT EXISTS glossary_terms_by_library
    ON glossary_terms(glossary_id, term_index);
