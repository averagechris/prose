use crate::model::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
    time::Duration,
};
use url::Url;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    Validation(String),
    #[error("{kind} `{id}` was not found")]
    NotFound { kind: &'static str, id: String },
    #[error("stale {kind} `{id}`: expected revision {expected}, current revision is {actual}")]
    Stale {
        kind: &'static str,
        id: String,
        expected: u64,
        actual: u64,
    },
    #[error("{0}")]
    Conflict(String),
    #[error("storage error: {0}")]
    Sql(#[from] rusqlite::Error),
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}

impl Error {
    pub fn code(&self) -> &'static str {
        match self {
            Self::Validation(_) | Self::Json(_) => "invalid_input",
            Self::NotFound { .. } => "not_found",
            Self::Stale { .. } => "stale_revision",
            Self::Conflict(_) => "conflict",
            Self::Sql(_) => "storage_error",
            Self::Io(_) => "io_error",
        }
    }
}

pub struct Store {
    conn: Connection,
    path: PathBuf,
}

static OPEN_LOCK: Mutex<()> = Mutex::new(());

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS schema_migrations (
 version INTEGER PRIMARY KEY, applied_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
INSERT OR IGNORE INTO schema_migrations(version) VALUES(1);
CREATE TABLE IF NOT EXISTS packs (id TEXT PRIMARY KEY) STRICT;
CREATE TABLE IF NOT EXISTS setting_events (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, key TEXT NOT NULL, value_json TEXT NOT NULL CHECK(json_valid(value_json)),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE TABLE IF NOT EXISTS pack_revisions (
 pack_id TEXT NOT NULL REFERENCES packs(id), revision INTEGER NOT NULL CHECK(revision > 0),
 description TEXT NOT NULL, tombstone INTEGER NOT NULL CHECK(tombstone IN (0,1)),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(pack_id, revision)
) STRICT;
CREATE TABLE IF NOT EXISTS context_revisions (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, name TEXT NOT NULL,
 description TEXT NOT NULL, content TEXT NOT NULL,
 PRIMARY KEY(pack_id, pack_revision, name),
 FOREIGN KEY(pack_id,pack_revision) REFERENCES pack_revisions(pack_id,revision)
) STRICT;
CREATE TABLE IF NOT EXISTS verb_revisions (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, name TEXT NOT NULL,
 description TEXT NOT NULL, family TEXT NOT NULL CHECK(family IN ('transform-selection','draft-from-intent')),
 instructions TEXT NOT NULL, length_unit TEXT CHECK(length_unit IN ('characters','words')),
 length_min INTEGER CHECK(length_min > 0), length_max INTEGER CHECK(length_max > 0),
 no_new_claims INTEGER NOT NULL CHECK(no_new_claims IN (0,1)),
 preserve_meaning INTEGER NOT NULL CHECK(preserve_meaning IN (0,1)),
 PRIMARY KEY(pack_id, pack_revision, name),
 CHECK((length_unit IS NULL) = (length_min IS NULL AND length_max IS NULL)),
 CHECK(length_min IS NULL OR length_max IS NULL OR length_min <= length_max),
 FOREIGN KEY(pack_id,pack_revision) REFERENCES pack_revisions(pack_id,revision)
) STRICT;
CREATE TABLE IF NOT EXISTS verb_context_binding_revisions (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, verb_name TEXT NOT NULL, context_name TEXT NOT NULL,
 PRIMARY KEY(pack_id,pack_revision,verb_name,context_name),
 FOREIGN KEY(pack_id,pack_revision,verb_name) REFERENCES verb_revisions(pack_id,pack_revision,name),
 FOREIGN KEY(pack_id,pack_revision,context_name) REFERENCES context_revisions(pack_id,pack_revision,name)
) STRICT;
CREATE TABLE IF NOT EXISTS audience_tier_revisions (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, name TEXT NOT NULL,
 description TEXT NOT NULL, requirements_json TEXT NOT NULL CHECK(json_valid(requirements_json) AND json_type(requirements_json)='array'),
 PRIMARY KEY(pack_id, pack_revision, name),
 FOREIGN KEY(pack_id,pack_revision) REFERENCES pack_revisions(pack_id,revision)
) STRICT;
CREATE TABLE IF NOT EXISTS surface_mapping_revisions (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, surface TEXT NOT NULL,
 context_name TEXT NOT NULL, default_audience_tier TEXT NOT NULL,
 PRIMARY KEY(pack_id, pack_revision, surface),
 FOREIGN KEY(pack_id,pack_revision) REFERENCES pack_revisions(pack_id,revision),
 FOREIGN KEY(pack_id,pack_revision,context_name)
   REFERENCES context_revisions(pack_id,pack_revision,name),
 FOREIGN KEY(pack_id,pack_revision,default_audience_tier)
   REFERENCES audience_tier_revisions(pack_id,pack_revision,name)
) STRICT;
CREATE TABLE IF NOT EXISTS drafts (
 id TEXT PRIMARY KEY,
 context_pack_id TEXT, context_pack_revision INTEGER, context_name TEXT,
 verb_pack_id TEXT, verb_pack_revision INTEGER, verb_name TEXT,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 CHECK((context_pack_id IS NULL) = (context_pack_revision IS NULL)),
 CHECK((context_pack_id IS NULL) = (context_name IS NULL)),
 CHECK((verb_pack_id IS NULL) = (verb_pack_revision IS NULL)),
 CHECK((verb_pack_id IS NULL) = (verb_name IS NULL)),
 FOREIGN KEY(context_pack_id,context_pack_revision,context_name)
   REFERENCES context_revisions(pack_id,pack_revision,name),
 FOREIGN KEY(verb_pack_id,verb_pack_revision,verb_name)
   REFERENCES verb_revisions(pack_id,pack_revision,name)
) STRICT;
CREATE TABLE IF NOT EXISTS draft_versions (
 draft_id TEXT NOT NULL REFERENCES drafts(id), version INTEGER NOT NULL CHECK(version > 0),
 parent_version INTEGER, content TEXT NOT NULL,
 author_kind TEXT NOT NULL CHECK(author_kind IN ('human','agent','capture','import')),
 provenance_json TEXT NOT NULL CHECK(json_valid(provenance_json) AND json_type(provenance_json)='object'),
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 PRIMARY KEY(draft_id,version),
 CHECK((version=1 AND parent_version IS NULL) OR (version>1 AND parent_version=version-1)),
 FOREIGN KEY(draft_id,parent_version) REFERENCES draft_versions(draft_id,version)
) STRICT;
CREATE TABLE IF NOT EXISTS attestation_batches (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
 human_approved INTEGER NOT NULL CHECK(human_approved=1),
 approved_by TEXT NOT NULL, via_harness TEXT NOT NULL, via_session TEXT NOT NULL,
 method TEXT NOT NULL CHECK(method IN ('verbal-lgtm','explicit')),
 tier_pack_id TEXT NOT NULL, tier_pack_revision INTEGER NOT NULL, tier_name TEXT NOT NULL,
 created_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 FOREIGN KEY(tier_pack_id,tier_pack_revision,tier_name)
   REFERENCES audience_tier_revisions(pack_id,pack_revision,name)
) STRICT;
CREATE TABLE IF NOT EXISTS attestations (
 id TEXT PRIMARY KEY, batch_id TEXT NOT NULL REFERENCES attestation_batches(id) DEFERRABLE INITIALLY DEFERRED,
 draft_id TEXT NOT NULL, draft_version INTEGER NOT NULL,
 decision TEXT NOT NULL CHECK(decision IN ('approved','excepted')),
 exceptions_json TEXT NOT NULL CHECK(json_valid(exceptions_json) AND json_type(exceptions_json)='array'),
 FOREIGN KEY(draft_id,draft_version) REFERENCES draft_versions(draft_id,version),
 UNIQUE(batch_id,draft_id,draft_version)
) STRICT;
CREATE TRIGGER IF NOT EXISTS packs_no_update BEFORE UPDATE ON packs BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE TRIGGER IF NOT EXISTS packs_no_delete BEFORE DELETE ON packs BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE TRIGGER IF NOT EXISTS settings_no_update BEFORE UPDATE ON setting_events BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS settings_no_delete BEFORE DELETE ON setting_events BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS pack_revisions_no_update BEFORE UPDATE ON pack_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS pack_revisions_no_delete BEFORE DELETE ON pack_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS context_revisions_no_update BEFORE UPDATE ON context_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS context_revisions_no_delete BEFORE DELETE ON context_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS verb_revisions_no_update BEFORE UPDATE ON verb_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS verb_revisions_no_delete BEFORE DELETE ON verb_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS verb_bindings_no_update BEFORE UPDATE ON verb_context_binding_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS verb_bindings_no_delete BEFORE DELETE ON verb_context_binding_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS tier_revisions_no_update BEFORE UPDATE ON audience_tier_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS tier_revisions_no_delete BEFORE DELETE ON audience_tier_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS tier_requirements_typed BEFORE INSERT ON audience_tier_revisions WHEN EXISTS(SELECT 1 FROM json_each(NEW.requirements_json) WHERE type!='text') BEGIN SELECT RAISE(ABORT,'tier requirements must be strings'); END;
CREATE TRIGGER IF NOT EXISTS mappings_no_update BEFORE UPDATE ON surface_mapping_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS mappings_no_delete BEFORE DELETE ON surface_mapping_revisions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS draft_versions_no_update BEFORE UPDATE ON draft_versions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS draft_versions_no_delete BEFORE DELETE ON draft_versions BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS drafts_no_update BEFORE UPDATE ON drafts BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE TRIGGER IF NOT EXISTS drafts_no_delete BEFORE DELETE ON drafts BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE TRIGGER IF NOT EXISTS attestations_no_update BEFORE UPDATE ON attestations BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE TRIGGER IF NOT EXISTS attestations_no_delete BEFORE DELETE ON attestations BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE TRIGGER IF NOT EXISTS attestation_exceptions_typed BEFORE INSERT ON attestations WHEN EXISTS(SELECT 1 FROM json_each(NEW.exceptions_json) WHERE type!='text' OR trim(value)='') BEGIN SELECT RAISE(ABORT,'attestation exceptions must be non-empty strings'); END;
CREATE TRIGGER IF NOT EXISTS attestation_batch_closed BEFORE INSERT ON attestations WHEN EXISTS(SELECT 1 FROM attestation_batches WHERE id=NEW.batch_id) BEGIN SELECT RAISE(ABORT,'attestation batch is closed'); END;
CREATE TRIGGER IF NOT EXISTS batches_no_update BEFORE UPDATE ON attestation_batches BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE TRIGGER IF NOT EXISTS batches_no_delete BEFORE DELETE ON attestation_batches BEGIN SELECT RAISE(ABORT,'immutable table'); END;
"#;
const SCHEMA_V2: &str = r#"
CREATE TABLE IF NOT EXISTS capture_events (
 sequence INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
 surface TEXT NOT NULL, url TEXT NOT NULL, content TEXT NOT NULL,
 draft_id TEXT, draft_version INTEGER,
 metadata_json TEXT NOT NULL CHECK(json_valid(metadata_json) AND json_type(metadata_json)='object'),
 captured_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now')),
 CHECK((draft_id IS NULL) = (draft_version IS NULL)),
 FOREIGN KEY(draft_id,draft_version) REFERENCES draft_versions(draft_id,version)
) STRICT;
CREATE TRIGGER IF NOT EXISTS captures_no_update BEFORE UPDATE ON capture_events BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE TRIGGER IF NOT EXISTS captures_no_delete BEFORE DELETE ON capture_events BEGIN SELECT RAISE(ABORT,'immutable table'); END;
INSERT OR IGNORE INTO schema_migrations(version) VALUES(2);
"#;
const SCHEMA_V3_PRE: &str = r#"
DROP TRIGGER IF EXISTS captures_no_update;
ALTER TABLE capture_events ADD COLUMN observation TEXT NOT NULL DEFAULT 'submit-attempt'
 CHECK(observation IN ('submit-attempt','surface-confirmed-post'));
ALTER TABLE capture_events ADD COLUMN parent_id TEXT REFERENCES capture_events(id)
 CHECK((observation='submit-attempt' AND parent_id IS NULL) OR
       (observation='surface-confirmed-post' AND parent_id IS NOT NULL));
ALTER TABLE capture_events ADD COLUMN normalized_url_key TEXT NOT NULL DEFAULT '';
"#;
const SCHEMA_V3: &str = r#"
CREATE TRIGGER IF NOT EXISTS captures_normalized_url_required BEFORE INSERT ON capture_events WHEN NEW.normalized_url_key='' BEGIN SELECT RAISE(ABORT,'capture normalized URL key is required'); END;
CREATE TRIGGER IF NOT EXISTS capture_confirmation_parent_distinct BEFORE INSERT ON capture_events WHEN NEW.observation='surface-confirmed-post' AND NEW.parent_id=NEW.id BEGIN SELECT RAISE(ABORT,'confirmation parent must be distinct'); END;
CREATE TRIGGER IF NOT EXISTS capture_confirmation_parent_exists BEFORE INSERT ON capture_events WHEN NEW.observation='surface-confirmed-post' AND NOT EXISTS(SELECT 1 FROM capture_events p WHERE p.id=NEW.parent_id) BEGIN SELECT RAISE(ABORT,'confirmation parent must exist'); END;
CREATE TRIGGER IF NOT EXISTS capture_confirmation_parent_attempt BEFORE INSERT ON capture_events WHEN NEW.observation='surface-confirmed-post' AND NOT EXISTS(SELECT 1 FROM capture_events p WHERE p.id=NEW.parent_id AND p.observation='submit-attempt') BEGIN SELECT RAISE(ABORT,'confirmation parent must be a submit-attempt'); END;
CREATE TRIGGER IF NOT EXISTS capture_confirmation_parent_target BEFORE INSERT ON capture_events WHEN NEW.observation='surface-confirmed-post' AND NOT EXISTS(SELECT 1 FROM capture_events p WHERE p.id=NEW.parent_id AND p.surface=NEW.surface AND p.normalized_url_key=NEW.normalized_url_key) BEGIN SELECT RAISE(ABORT,'confirmation must match parent surface and normalized URL'); END;
CREATE TRIGGER IF NOT EXISTS captures_no_update BEFORE UPDATE ON capture_events BEGIN SELECT RAISE(ABORT,'immutable table'); END;
CREATE UNIQUE INDEX IF NOT EXISTS attestations_exact_draft
 ON attestations(id,draft_id,draft_version);
CREATE TABLE IF NOT EXISTS pack_item_origins (
 id INTEGER PRIMARY KEY AUTOINCREMENT,
 kind TEXT NOT NULL CHECK(kind IN ('approved-draft','external-import')),
 recorded_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
) STRICT;
CREATE TABLE IF NOT EXISTS approved_draft_origins (
 origin_id INTEGER PRIMARY KEY REFERENCES pack_item_origins(id),
 attestation_id TEXT NOT NULL, draft_id TEXT NOT NULL, draft_version INTEGER NOT NULL,
 FOREIGN KEY(attestation_id,draft_id,draft_version)
   REFERENCES attestations(id,draft_id,draft_version),
 FOREIGN KEY(draft_id,draft_version) REFERENCES draft_versions(draft_id,version)
) STRICT;
CREATE TABLE IF NOT EXISTS external_import_origins (
 origin_id INTEGER PRIMARY KEY REFERENCES pack_item_origins(id),
 source_label TEXT, source_uri TEXT,
 material_sha256 TEXT NOT NULL CHECK(length(material_sha256)=64 AND material_sha256 NOT GLOB '*[^0-9a-f]*'),
 channel TEXT NOT NULL CHECK(length(trim(channel)) > 0)
) STRICT;
CREATE TABLE IF NOT EXISTS context_revision_origins (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, name TEXT NOT NULL,
 origin_id INTEGER NOT NULL REFERENCES pack_item_origins(id),
 PRIMARY KEY(pack_id,pack_revision,name),
 FOREIGN KEY(pack_id,pack_revision,name) REFERENCES context_revisions(pack_id,pack_revision,name)
) STRICT;
CREATE TABLE IF NOT EXISTS verb_revision_origins (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, name TEXT NOT NULL,
 origin_id INTEGER NOT NULL REFERENCES pack_item_origins(id),
 PRIMARY KEY(pack_id,pack_revision,name),
 FOREIGN KEY(pack_id,pack_revision,name) REFERENCES verb_revisions(pack_id,pack_revision,name)
) STRICT;
CREATE TABLE IF NOT EXISTS audience_tier_revision_origins (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, name TEXT NOT NULL,
 origin_id INTEGER NOT NULL REFERENCES pack_item_origins(id),
 PRIMARY KEY(pack_id,pack_revision,name),
 FOREIGN KEY(pack_id,pack_revision,name) REFERENCES audience_tier_revisions(pack_id,pack_revision,name)
) STRICT;
CREATE TABLE IF NOT EXISTS surface_mapping_revision_origins (
 pack_id TEXT NOT NULL, pack_revision INTEGER NOT NULL, surface TEXT NOT NULL,
 origin_id INTEGER NOT NULL REFERENCES pack_item_origins(id),
 PRIMARY KEY(pack_id,pack_revision,surface),
 FOREIGN KEY(pack_id,pack_revision,surface) REFERENCES surface_mapping_revisions(pack_id,pack_revision,surface)
) STRICT;
CREATE TRIGGER IF NOT EXISTS approved_origin_requires_approval BEFORE INSERT ON approved_draft_origins
 WHEN NOT EXISTS(SELECT 1 FROM attestations a WHERE a.id=NEW.attestation_id AND a.draft_id=NEW.draft_id AND a.draft_version=NEW.draft_version AND a.decision='approved')
 BEGIN SELECT RAISE(ABORT,'approved-draft origin requires exact approved attestation'); END;
CREATE TRIGGER IF NOT EXISTS approved_origin_kind BEFORE INSERT ON approved_draft_origins
 WHEN NOT EXISTS(SELECT 1 FROM pack_item_origins o WHERE o.id=NEW.origin_id AND o.kind='approved-draft')
 BEGIN SELECT RAISE(ABORT,'approved-draft origin kind mismatch'); END;
CREATE TRIGGER IF NOT EXISTS external_origin_kind BEFORE INSERT ON external_import_origins
 WHEN NOT EXISTS(SELECT 1 FROM pack_item_origins o WHERE o.id=NEW.origin_id AND o.kind='external-import')
 BEGIN SELECT RAISE(ABORT,'external-import origin kind mismatch'); END;
CREATE TRIGGER IF NOT EXISTS context_origin_complete BEFORE INSERT ON context_revision_origins
 WHEN NOT EXISTS(SELECT 1 FROM pack_item_origins o LEFT JOIN approved_draft_origins a ON a.origin_id=o.id LEFT JOIN external_import_origins e ON e.origin_id=o.id WHERE o.id=NEW.origin_id AND ((o.kind='approved-draft' AND a.origin_id IS NOT NULL) OR (o.kind='external-import' AND e.origin_id IS NOT NULL)))
 BEGIN SELECT RAISE(ABORT,'incomplete pack-item origin'); END;
CREATE TRIGGER IF NOT EXISTS context_approved_material BEFORE INSERT ON context_revision_origins
 WHEN EXISTS(SELECT 1 FROM pack_item_origins o WHERE o.id=NEW.origin_id AND o.kind='approved-draft') AND NOT EXISTS(SELECT 1 FROM context_revisions c JOIN approved_draft_origins a ON a.origin_id=NEW.origin_id JOIN draft_versions d ON d.draft_id=a.draft_id AND d.version=a.draft_version WHERE c.pack_id=NEW.pack_id AND c.pack_revision=NEW.pack_revision AND c.name=NEW.name AND c.content=d.content)
 BEGIN SELECT RAISE(ABORT,'approved context must match exact draft'); END;
CREATE TRIGGER IF NOT EXISTS verb_origin_complete BEFORE INSERT ON verb_revision_origins
 WHEN NOT EXISTS(SELECT 1 FROM pack_item_origins o LEFT JOIN approved_draft_origins a ON a.origin_id=o.id LEFT JOIN external_import_origins e ON e.origin_id=o.id WHERE o.id=NEW.origin_id AND ((o.kind='approved-draft' AND a.origin_id IS NOT NULL) OR (o.kind='external-import' AND e.origin_id IS NOT NULL)))
 BEGIN SELECT RAISE(ABORT,'incomplete pack-item origin'); END;
CREATE TRIGGER IF NOT EXISTS verb_approved_material BEFORE INSERT ON verb_revision_origins
 WHEN EXISTS(SELECT 1 FROM pack_item_origins o WHERE o.id=NEW.origin_id AND o.kind='approved-draft') AND NOT EXISTS(SELECT 1 FROM verb_revisions v JOIN approved_draft_origins a ON a.origin_id=NEW.origin_id JOIN draft_versions d ON d.draft_id=a.draft_id AND d.version=a.draft_version WHERE v.pack_id=NEW.pack_id AND v.pack_revision=NEW.pack_revision AND v.name=NEW.name AND v.instructions=d.content)
 BEGIN SELECT RAISE(ABORT,'approved verb must match exact draft'); END;
CREATE TRIGGER IF NOT EXISTS tier_origin_complete BEFORE INSERT ON audience_tier_revision_origins
 WHEN NOT EXISTS(SELECT 1 FROM pack_item_origins o LEFT JOIN approved_draft_origins a ON a.origin_id=o.id LEFT JOIN external_import_origins e ON e.origin_id=o.id WHERE o.id=NEW.origin_id AND ((o.kind='approved-draft' AND a.origin_id IS NOT NULL) OR (o.kind='external-import' AND e.origin_id IS NOT NULL)))
 BEGIN SELECT RAISE(ABORT,'incomplete pack-item origin'); END;
CREATE TRIGGER IF NOT EXISTS tier_rejects_approved_origin BEFORE INSERT ON audience_tier_revision_origins WHEN EXISTS(SELECT 1 FROM pack_item_origins o WHERE o.id=NEW.origin_id AND o.kind='approved-draft') BEGIN SELECT RAISE(ABORT,'approved-draft origin does not support tier items'); END;
CREATE TRIGGER IF NOT EXISTS surface_origin_complete BEFORE INSERT ON surface_mapping_revision_origins
 WHEN NOT EXISTS(SELECT 1 FROM pack_item_origins o LEFT JOIN approved_draft_origins a ON a.origin_id=o.id LEFT JOIN external_import_origins e ON e.origin_id=o.id WHERE o.id=NEW.origin_id AND ((o.kind='approved-draft' AND a.origin_id IS NOT NULL) OR (o.kind='external-import' AND e.origin_id IS NOT NULL)))
 BEGIN SELECT RAISE(ABORT,'incomplete pack-item origin'); END;
CREATE TRIGGER IF NOT EXISTS surface_rejects_approved_origin BEFORE INSERT ON surface_mapping_revision_origins WHEN EXISTS(SELECT 1 FROM pack_item_origins o WHERE o.id=NEW.origin_id AND o.kind='approved-draft') BEGIN SELECT RAISE(ABORT,'approved-draft origin does not support surface items'); END;
CREATE TRIGGER IF NOT EXISTS origins_no_update BEFORE UPDATE ON pack_item_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS origins_no_delete BEFORE DELETE ON pack_item_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS approved_origins_no_update BEFORE UPDATE ON approved_draft_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS approved_origins_no_delete BEFORE DELETE ON approved_draft_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS external_origins_no_update BEFORE UPDATE ON external_import_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS external_origins_no_delete BEFORE DELETE ON external_import_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS context_origins_no_update BEFORE UPDATE ON context_revision_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS context_origins_no_delete BEFORE DELETE ON context_revision_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS verb_origins_no_update BEFORE UPDATE ON verb_revision_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS verb_origins_no_delete BEFORE DELETE ON verb_revision_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS tier_origins_no_update BEFORE UPDATE ON audience_tier_revision_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS tier_origins_no_delete BEFORE DELETE ON audience_tier_revision_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS surface_origins_no_update BEFORE UPDATE ON surface_mapping_revision_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
CREATE TRIGGER IF NOT EXISTS surface_origins_no_delete BEFORE DELETE ON surface_mapping_revision_origins BEGIN SELECT RAISE(ABORT,'append-only table'); END;
INSERT OR IGNORE INTO schema_migrations(version) VALUES(3);
"#;
const APPLICATION_ID: u32 = 0x5052_4f53;
const SCHEMA_VERSION: u32 = 3;

impl Store {
    pub fn default_path() -> Result<PathBuf> {
        if let Some(path) = std::env::var_os("XDG_DATA_HOME").filter(|value| !value.is_empty()) {
            return Ok(PathBuf::from(path).join("prose/prose.db"));
        }
        let home = std::env::var_os("HOME")
            .filter(|value| !value.is_empty())
            .ok_or_else(|| {
                Error::Validation("HOME or XDG_DATA_HOME must be set (or pass --store)".into())
            })?;
        Ok(PathBuf::from(home).join(".local/share/prose/prose.db"))
    }

    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        // SQLite serializes database writes, but first-open schema and WAL
        // initialization spans multiple pragmas and transactions. Keep those
        // connection setup steps from racing within this process.
        let _open_guard = OPEN_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let path = path.as_ref().to_path_buf();
        if fs::symlink_metadata(&path).is_ok_and(|metadata| metadata.file_type().is_symlink()) {
            return Err(Error::Validation(
                "store path must not be a symbolic link".into(),
            ));
        }
        let parent = path
            .parent()
            .filter(|value| !value.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        prepare_store_dir(parent)?;
        prepare_store_file(&path)?;
        private_sqlite_sidecars(&path)?;
        let mut conn = Connection::open(&path)?;
        private_file(&path)?;
        conn.busy_timeout(Duration::from_secs(5))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        let version: u32 = conn.pragma_query_value(None, "user_version", |row| row.get(0))?;
        if version > SCHEMA_VERSION {
            return Err(Error::Conflict(format!(
                "store schema version {version} is newer than supported version {SCHEMA_VERSION}"
            )));
        }
        if version == 0 {
            let objects: u64 = conn.query_row(
                "SELECT count(*) FROM sqlite_schema WHERE name NOT LIKE 'sqlite_%'",
                [],
                |row| row.get(0),
            )?;
            if objects != 0 {
                return Err(Error::Conflict(
                    "store has application objects but no recognized schema version".into(),
                ));
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA)?;
            tx.execute_batch(SCHEMA_V2)?;
            apply_schema_v3(&tx)?;
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        } else if version == 1 {
            let application_id: u32 =
                conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
            if application_id != APPLICATION_ID {
                return Err(Error::Conflict(
                    "store schema identity is missing or unrecognized".into(),
                ));
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            tx.execute_batch(SCHEMA_V2)?;
            apply_schema_v3(&tx)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        } else if version == 2 {
            let application_id: u32 =
                conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
            if application_id != APPLICATION_ID {
                return Err(Error::Conflict(
                    "store schema identity is missing or unrecognized".into(),
                ));
            }
            let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
            apply_schema_v3(&tx)?;
            tx.pragma_update(None, "user_version", SCHEMA_VERSION)?;
            tx.commit()?;
        }
        verify_schema(&conn)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        private_sqlite_sidecars(&path)?;
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
    pub fn journal_mode(&self) -> Result<String> {
        Ok(self
            .conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))?)
    }
    pub fn foreign_keys_enabled(&self) -> Result<bool> {
        Ok(self
            .conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))?)
    }
    pub fn schema_version(&self) -> Result<u32> {
        Ok(self
            .conn
            .pragma_query_value(None, "user_version", |row| row.get(0))?)
    }
    pub fn create_pack(&mut self, doc: &PackDocument) -> Result<Pack> {
        self.create_pack_external(doc, "store-create")
    }

    pub fn create_pack_external(&mut self, doc: &PackDocument, channel: &str) -> Result<Pack> {
        validate_origin_channel(channel)?;
        let doc = canonical_pack(doc);
        doc.validate().map_err(Error::Validation)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM packs WHERE id=?1)",
            [&doc.id],
            |r| r.get::<_, bool>(0),
        )? {
            return Err(Error::Conflict(format!("pack `{}` already exists", doc.id)));
        }
        tx.execute("INSERT INTO packs(id) VALUES(?1)", [&doc.id])?;
        insert_pack_revision(&tx, &doc, 1, false)?;
        populate_revision_origins(&tx, &doc, 1, None, channel, None)?;
        tx.commit()?;
        Ok(Pack {
            revision: 1,
            deleted: false,
            document: doc,
        })
    }

    pub fn update_pack(&mut self, doc: &PackDocument, expected: u64) -> Result<Pack> {
        self.update_pack_external(doc, expected, "store-update")
    }

    pub fn update_pack_external(
        &mut self,
        doc: &PackDocument,
        expected: u64,
        channel: &str,
    ) -> Result<Pack> {
        validate_origin_channel(channel)?;
        validate_revision(expected, "expected revision")?;
        let doc = canonical_pack(doc);
        doc.validate().map_err(Error::Validation)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (actual, deleted) = latest_revision(&tx, &doc.id)?.ok_or_else(|| Error::NotFound {
            kind: "pack",
            id: doc.id.clone(),
        })?;
        if actual != expected {
            return Err(Error::Stale {
                kind: "pack",
                id: doc.id.clone(),
                expected,
                actual,
            });
        }
        if deleted {
            return Err(Error::NotFound {
                kind: "pack",
                id: doc.id.clone(),
            });
        }
        if actual == i64::MAX as u64 {
            return Err(Error::Conflict(format!(
                "pack `{}` has reached the maximum revision",
                doc.id
            )));
        }
        let old = load_pack(&tx, &doc.id, actual, false)?;
        let revision = actual + 1;
        insert_pack_revision(&tx, &doc, revision, false)?;
        populate_revision_origins(
            &tx,
            &doc,
            revision,
            Some((actual, &old.document)),
            channel,
            None,
        )?;
        tx.commit()?;
        Ok(Pack {
            revision,
            deleted: false,
            document: doc,
        })
    }

    /// Revise one item and attach schema-v3 external-import provenance atomically.
    #[allow(clippy::too_many_arguments)]
    pub fn update_pack_item_external(
        &mut self,
        doc: &PackDocument,
        expected: u64,
        kind: ItemKind,
        key: &str,
        _caller_material: &serde_json::Value,
        source_label: Option<&str>,
        source_uri: Option<&str>,
        channel: &str,
    ) -> Result<Pack> {
        if channel.trim().is_empty() {
            return Err(Error::Validation("origin channel must not be empty".into()));
        }
        self.update_pack_item_origin(
            doc,
            expected,
            kind,
            key,
            Some(NewOrigin::External {
                source_label,
                source_uri,
                channel,
            }),
        )
    }

    /// Return the exact approved draft named by an attestation row ID.
    pub fn approved_draft(
        &self,
        attestation_id: &str,
        source: &DraftTarget,
    ) -> Result<DraftVersion> {
        validate_revision(source.version, "draft version")?;
        if !super_key(&source.draft_id) {
            return Err(Error::Validation(
                "draft id has invalid characters or length".into(),
            ));
        }
        self
            .conn
            .query_row(
                "SELECT 1 FROM attestations WHERE id=?1 AND decision='approved' AND draft_id=?2 AND draft_version=?3",
                params![attestation_id, source.draft_id, source.version],
                |_| Ok(()),
            )
            .optional()?
            .ok_or_else(|| Error::NotFound {
                kind: "exact approved attestation",
                id: format!("{attestation_id} for {}@{}", source.draft_id, source.version),
            })?;
        get_draft_from(&self.conn, &source.draft_id, Some(source.version))
    }

    /// Revise one item and attach provenance to an exact approved attestation.
    pub fn update_pack_item_approved(
        &mut self,
        doc: &PackDocument,
        expected: u64,
        kind: ItemKind,
        key: &str,
        attestation_id: &str,
        source: &DraftTarget,
    ) -> Result<Pack> {
        if matches!(kind, ItemKind::Tier | ItemKind::Surface) {
            return Err(Error::Validation(
                "approved-draft installs support only context and verb items".into(),
            ));
        }
        validate_revision(expected, "expected revision")?;
        validate_revision(source.version, "draft version")?;
        if !super_key(&source.draft_id) {
            return Err(Error::Validation(
                "draft id has invalid characters or length".into(),
            ));
        }
        let mut doc = canonical_pack(doc);
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.query_row(
            "SELECT 1 FROM attestations WHERE id=?1 AND decision='approved' AND draft_id=?2 AND draft_version=?3",
            params![attestation_id, source.draft_id, source.version],
            |_| Ok(()),
        )
        .optional()?
        .ok_or_else(|| Error::NotFound {
            kind: "exact approved attestation",
            id: format!("{attestation_id} for {}@{}", source.draft_id, source.version),
        })?;
        let draft = get_draft_from(&tx, &source.draft_id, Some(source.version))?;
        match kind {
            ItemKind::Context => {
                doc.contexts
                    .iter_mut()
                    .find(|item| item.name == key)
                    .ok_or_else(|| Error::NotFound {
                        kind: "context",
                        id: key.into(),
                    })?
                    .content = draft.content;
            }
            ItemKind::Verb => {
                doc.verbs
                    .iter_mut()
                    .find(|item| item.name == key)
                    .ok_or_else(|| Error::NotFound {
                        kind: "verb",
                        id: key.into(),
                    })?
                    .instructions = draft.content;
            }
            ItemKind::Tier | ItemKind::Surface => unreachable!("validated above"),
        }
        doc.validate().map_err(Error::Validation)?;
        let (actual, deleted) = latest_revision(&tx, &doc.id)?.ok_or_else(|| Error::NotFound {
            kind: "pack",
            id: doc.id.clone(),
        })?;
        if actual != expected {
            return Err(Error::Stale {
                kind: "pack",
                id: doc.id.clone(),
                expected,
                actual,
            });
        }
        if deleted {
            return Err(Error::NotFound {
                kind: "pack",
                id: doc.id.clone(),
            });
        }
        let revision = actual.checked_add(1).ok_or_else(|| {
            Error::Conflict(format!(
                "pack `{}` has reached the maximum revision",
                doc.id
            ))
        })?;
        let old = load_pack(&tx, &doc.id, actual, false)?;
        insert_pack_revision(&tx, &doc, revision, false)?;
        populate_revision_origins(
            &tx,
            &doc,
            revision,
            Some((actual, &old.document)),
            "approved-draft-install",
            Some((
                kind,
                key,
                NewOrigin::Approved {
                    attestation_id,
                    draft_id: &source.draft_id,
                    draft_version: source.version,
                },
            )),
        )?;
        tx.commit()?;
        Ok(Pack {
            revision,
            deleted: false,
            document: doc,
        })
    }

    /// Revise a snapshot after deleting one item while retaining every surviving origin link.
    pub fn update_pack_item_deleted(
        &mut self,
        doc: &PackDocument,
        expected: u64,
        kind: ItemKind,
        key: &str,
    ) -> Result<Pack> {
        self.update_pack_item_origin(doc, expected, kind, key, None)
    }

    fn update_pack_item_origin(
        &mut self,
        doc: &PackDocument,
        expected: u64,
        kind: ItemKind,
        key: &str,
        origin: Option<NewOrigin<'_>>,
    ) -> Result<Pack> {
        validate_revision(expected, "expected revision")?;
        let doc = canonical_pack(doc);
        doc.validate().map_err(Error::Validation)?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (actual, deleted) = latest_revision(&tx, &doc.id)?.ok_or_else(|| Error::NotFound {
            kind: "pack",
            id: doc.id.clone(),
        })?;
        if actual != expected {
            return Err(Error::Stale {
                kind: "pack",
                id: doc.id.clone(),
                expected,
                actual,
            });
        }
        if deleted {
            return Err(Error::NotFound {
                kind: "pack",
                id: doc.id.clone(),
            });
        }
        let revision = actual.checked_add(1).ok_or_else(|| {
            Error::Conflict(format!(
                "pack `{}` has reached the maximum revision",
                doc.id
            ))
        })?;
        let old = load_pack(&tx, &doc.id, actual, false)?;
        insert_pack_revision(&tx, &doc, revision, false)?;
        let fallback_channel = match origin {
            Some(NewOrigin::External { channel, .. }) => channel,
            Some(NewOrigin::Approved { .. }) => "approved-draft-install",
            None => "item-delete",
        };
        populate_revision_origins(
            &tx,
            &doc,
            revision,
            Some((actual, &old.document)),
            fallback_channel,
            origin.map(|value| (kind, key, value)),
        )?;
        tx.commit()?;
        Ok(Pack {
            revision,
            deleted: false,
            document: doc,
        })
    }

    pub fn item_origin(
        &self,
        pack_id: &str,
        revision: u64,
        kind: ItemKind,
        key: &str,
    ) -> Result<Option<PackItemOrigin>> {
        let link = origin_link_table(kind);
        let key_column = origin_key_column(kind);
        let sql = format!(
            "SELECT o.kind,o.recorded_at,a.attestation_id,a.draft_id,a.draft_version,e.source_label,e.source_uri,e.material_sha256,e.channel FROM {link} l JOIN pack_item_origins o ON o.id=l.origin_id LEFT JOIN approved_draft_origins a ON a.origin_id=o.id LEFT JOIN external_import_origins e ON e.origin_id=o.id WHERE l.pack_id=?1 AND l.pack_revision=?2 AND l.{key_column}=?3"
        );
        Ok(self
            .conn
            .query_row(&sql, params![pack_id, revision, key], origin_row)
            .optional()?)
    }

    pub fn delete_pack(&mut self, id: &str, expected: u64) -> Result<Pack> {
        validate_revision(expected, "expected revision")?;
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (actual, deleted) = latest_revision(&tx, id)?.ok_or_else(|| Error::NotFound {
            kind: "pack",
            id: id.into(),
        })?;
        if actual != expected {
            return Err(Error::Stale {
                kind: "pack",
                id: id.into(),
                expected,
                actual,
            });
        }
        if deleted {
            return Err(Error::NotFound {
                kind: "pack",
                id: id.into(),
            });
        }
        if actual == i64::MAX as u64 {
            return Err(Error::Conflict(format!(
                "pack `{id}` has reached the maximum revision"
            )));
        }
        let old = load_pack(&tx, id, actual, false)?;
        insert_pack_revision(&tx, &old.document, actual + 1, true)?;
        populate_revision_origins(
            &tx,
            &old.document,
            actual + 1,
            Some((actual, &old.document)),
            "pack-delete",
            None,
        )?;
        tx.commit()?;
        Ok(Pack {
            revision: actual + 1,
            deleted: true,
            document: old.document,
        })
    }

    pub fn get_pack(&self, id: &str, revision: Option<u64>) -> Result<Pack> {
        validate_optional_revision(revision, "pack revision")?;
        let (rev, deleted) = match revision {
            Some(v) => (
                v,
                self.conn
                    .query_row(
                        "SELECT tombstone FROM pack_revisions WHERE pack_id=?1 AND revision=?2",
                        params![id, v],
                        |r| r.get(0),
                    )
                    .optional()?
                    .ok_or_else(|| Error::NotFound {
                        kind: "pack revision",
                        id: format!("{id}@{v}"),
                    })?,
            ),
            None => latest_revision(&self.conn, id)?.ok_or_else(|| Error::NotFound {
                kind: "pack",
                id: id.into(),
            })?,
        };
        load_pack(&self.conn, id, rev, deleted)
    }

    pub fn list_packs(&self) -> Result<Vec<PackSummary>> {
        let mut stmt = self.conn.prepare("SELECT p.pack_id,p.revision,p.description FROM pack_revisions p JOIN (SELECT pack_id,max(revision) revision FROM pack_revisions GROUP BY pack_id) x USING(pack_id,revision) WHERE p.tombstone=0 ORDER BY p.pack_id")?;
        Ok(stmt
            .query_map([], |r| {
                Ok(PackSummary {
                    id: r.get(0)?,
                    revision: r.get(1)?,
                    description: r.get(2)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn use_pack(&mut self, id: &str) -> Result<PackSummary> {
        let revision = active_pack_revision(&self.conn, id)?;
        let pack = self.get_pack(id, Some(revision))?;
        let value = serde_json::to_string(id)?;
        self.conn.execute(
            "INSERT INTO setting_events(key,value_json) VALUES('active_pack',?1)",
            [value],
        )?;
        Ok(PackSummary {
            id: id.into(),
            revision,
            description: pack.document.description,
        })
    }

    pub fn active_pack(&self) -> Result<Option<String>> {
        let value: Option<String> = self.conn.query_row(
            "SELECT value_json FROM setting_events WHERE key='active_pack' ORDER BY sequence DESC LIMIT 1",
            [],
            |row| row.get(0),
        ).optional()?;
        value
            .map(|value| serde_json::from_str(&value).map_err(Error::Json))
            .transpose()
    }

    pub fn serve_context(
        &self,
        name: &str,
        pack_id: Option<&str>,
    ) -> Result<(String, u64, ContextSpec)> {
        let selected = self.selected_pack(pack_id)?;
        self.serve_named_context(name, Some(&selected))
    }
    pub fn serve_verb(&self, name: &str, pack_id: Option<&str>) -> Result<(String, u64, VerbSpec)> {
        let selected = self.selected_pack(pack_id)?;
        self.serve_named_verb(name, Some(&selected))
    }

    pub fn serve_surface(&self, surface: &str, pack: Option<&str>) -> Result<SurfaceResolution> {
        let selected = self.selected_pack(pack)?;
        let mut stmt = self.conn.prepare(
            "SELECT m.pack_id,m.pack_revision,m.surface,c.name,c.description,c.content,t.name,t.description,t.requirements_json FROM surface_mapping_revisions m JOIN (SELECT pack_id,max(revision) revision FROM pack_revisions GROUP BY pack_id) x ON x.pack_id=m.pack_id AND x.revision=m.pack_revision JOIN pack_revisions p ON p.pack_id=x.pack_id AND p.revision=x.revision JOIN context_revisions c ON c.pack_id=m.pack_id AND c.pack_revision=m.pack_revision AND c.name=m.context_name JOIN audience_tier_revisions t ON t.pack_id=m.pack_id AND t.pack_revision=m.pack_revision AND t.name=m.default_audience_tier WHERE p.tombstone=0 AND m.surface=?1 AND (?2 IS NULL OR m.pack_id=?2) ORDER BY m.pack_id",
        )?;
        let rows = stmt
            .query_map(params![surface, selected], |r| {
                Ok(SurfaceResolution {
                    pack_id: r.get(0)?,
                    pack_revision: r.get(1)?,
                    surface: r.get(2)?,
                    context: ContextSpec {
                        name: r.get(3)?,
                        description: r.get(4)?,
                        content: r.get(5)?,
                    },
                    default_audience_tier: AudienceTier {
                        name: r.get(6)?,
                        description: r.get(7)?,
                        requirements: json_column(r, 8)?,
                    },
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if rows.is_empty() {
            Err(Error::NotFound {
                kind: "surface",
                id: surface.into(),
            })
        } else if rows.len() > 1 {
            Err(Error::Conflict(format!(
                "surface `{surface}` is ambiguous; specify --pack"
            )))
        } else {
            Ok(rows.into_iter().next().expect("one row was checked"))
        }
    }

    pub fn list_verbs(
        &self,
        context: Option<&str>,
        pack: Option<&str>,
    ) -> Result<Vec<(String, u64, VerbSpec)>> {
        let selected = self.selected_pack(pack)?;
        let revision = active_pack_revision(&self.conn, &selected)?;
        let document = self.get_pack(&selected, Some(revision))?.document;
        Ok(document
            .verbs
            .into_iter()
            .filter(|verb| {
                context
                    .is_none_or(|name| verb.context_bindings.iter().any(|binding| binding == name))
            })
            .map(|verb| (selected.clone(), revision, verb))
            .collect())
    }

    fn selected_pack(&self, explicit: Option<&str>) -> Result<String> {
        selected_pack_from(&self.conn, explicit)
    }

    /// Resolves all assist material and records its capture-authored source in
    /// one write transaction. A supplied draft must already carry context and
    /// verb refs exactly matching the selected pack revision; missing,
    /// historical, or otherwise different refs are rejected because draft refs
    /// are immutable for the whole draft chain.
    pub fn prepare_assist(&mut self, input: &AssistPreparation) -> Result<PreparedAssist> {
        if input.selection.trim().is_empty() {
            return Err(Error::Validation("selection must not be empty".into()));
        }
        let provenance = serde_json::json!({
            "surface": input.surface,
            "surrounding_context": input.surrounding_context,
        });
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let pack_id = selected_pack_from(&tx, input.pack.as_deref())?;
        let pack_revision = active_pack_revision(&tx, &pack_id)?;
        let document = load_pack(&tx, &pack_id, pack_revision, false)?.document;
        let context = document
            .contexts
            .into_iter()
            .find(|context| context.name == input.context)
            .ok_or_else(|| Error::NotFound {
                kind: "context",
                id: format!("{pack_id}/{}", input.context),
            })?;
        let verb = document
            .verbs
            .into_iter()
            .find(|verb| verb.name == input.verb)
            .ok_or_else(|| Error::NotFound {
                kind: "verb",
                id: format!("{pack_id}/{}", input.verb),
            })?;
        if !verb.context_bindings.is_empty()
            && !verb
                .context_bindings
                .iter()
                .any(|binding| binding == &context.name)
        {
            return Err(Error::Validation(format!(
                "verb `{}` is not bound to context `{}`",
                verb.name, context.name
            )));
        }
        let context_ref = RevisionRef {
            pack_id: pack_id.clone(),
            pack_revision,
            name: context.name.clone(),
        };
        let verb_ref = RevisionRef {
            pack_id: pack_id.clone(),
            pack_revision,
            name: verb.name.clone(),
        };

        let source = if let Some(target) = &input.draft {
            validate_revision(target.version, "draft version")?;
            let supplied = get_draft_from(&tx, &target.draft_id, Some(target.version))?;
            if supplied.context.as_ref() != Some(&context_ref)
                || supplied.verb.as_ref() != Some(&verb_ref)
            {
                return Err(Error::Validation(format!(
                    "draft `{}@{}` context and verb refs do not match assist material",
                    target.draft_id, target.version
                )));
            }
            let actual: u64 = tx.query_row(
                "SELECT max(version) FROM draft_versions WHERE draft_id=?1",
                [&target.draft_id],
                |row| row.get(0),
            )?;
            if actual != target.version {
                return Err(Error::Stale {
                    kind: "draft",
                    id: target.draft_id.clone(),
                    expected: target.version,
                    actual,
                });
            }
            if actual == i64::MAX as u64 {
                return Err(Error::Conflict(format!(
                    "draft `{}` has reached the maximum version",
                    target.draft_id
                )));
            }
            tx.execute(
                "INSERT INTO draft_versions(draft_id,version,parent_version,content,author_kind,provenance_json) VALUES(?1,?2,?3,?4,?5,?6)",
                params![target.draft_id, actual + 1, actual, input.selection, author_kind_db(AuthorKind::Capture), serde_json::to_string(&provenance)?],
            )?;
            DraftTarget {
                draft_id: target.draft_id.clone(),
                version: actual + 1,
            }
        } else {
            let id = new_id(&tx)?;
            tx.execute(
                "INSERT INTO drafts(id,context_pack_id,context_pack_revision,context_name,verb_pack_id,verb_pack_revision,verb_name) VALUES(?1,?2,?3,?4,?5,?6,?7)",
                params![id, pack_id, pack_revision, context.name, pack_id, pack_revision, verb.name],
            )?;
            tx.execute(
                "INSERT INTO draft_versions(draft_id,version,parent_version,content,author_kind,provenance_json) VALUES(?1,1,NULL,?2,?3,?4)",
                params![id, input.selection, author_kind_db(AuthorKind::Capture), serde_json::to_string(&provenance)?],
            )?;
            DraftTarget {
                draft_id: id,
                version: 1,
            }
        };
        tx.commit()?;
        Ok(PreparedAssist {
            pack_id,
            pack_revision,
            context: context_ref,
            context_text: context.content,
            verb: verb_ref,
            verb_instructions: verb.instructions,
            source,
        })
    }

    pub fn create_draft(&mut self, input: &DraftCreate) -> Result<DraftVersion> {
        if input.content.trim().is_empty() {
            return Err(Error::Validation("draft content must not be empty".into()));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let id = match &input.id {
            Some(id) => id.clone(),
            None => new_id(&tx)?,
        };
        if !super_key(&id) {
            return Err(Error::Validation(
                "draft id has invalid characters or length".into(),
            ));
        }
        let context_revision = input
            .context
            .as_ref()
            .map(|value| resolve_ref(&tx, value, RefKind::Context))
            .transpose()?;
        let verb_revision = input
            .verb
            .as_ref()
            .map(|value| resolve_ref(&tx, value, RefKind::Verb))
            .transpose()?;
        let changed=tx.execute("INSERT OR IGNORE INTO drafts(id,context_pack_id,context_pack_revision,context_name,verb_pack_id,verb_pack_revision,verb_name) VALUES(?1,?2,?3,?4,?5,?6,?7)",params![id,input.context.as_ref().map(|v|&v.pack_id),context_revision,input.context.as_ref().map(|v|&v.name),input.verb.as_ref().map(|v|&v.pack_id),verb_revision,input.verb.as_ref().map(|v|&v.name)])?;
        if changed == 0 {
            return Err(Error::Conflict(format!("draft `{id}` already exists")));
        }
        if !input.provenance.is_object() {
            return Err(Error::Validation(
                "draft provenance must be a JSON object".into(),
            ));
        }
        tx.execute("INSERT INTO draft_versions(draft_id,version,parent_version,content,author_kind,provenance_json) VALUES(?1,1,NULL,?2,?3,?4)",params![id,input.content,author_kind_db(input.author_kind),serde_json::to_string(&input.provenance)?])?;
        tx.commit()?;
        self.get_draft(&id, Some(1))
    }

    pub fn revise_draft(&mut self, id: &str, expected: u64, content: &str) -> Result<DraftVersion> {
        self.revise_draft_with(
            id,
            expected,
            content,
            AuthorKind::Human,
            &serde_json::json!({}),
        )
    }

    pub fn revise_draft_with(
        &mut self,
        id: &str,
        expected: u64,
        content: &str,
        author_kind: AuthorKind,
        provenance: &serde_json::Value,
    ) -> Result<DraftVersion> {
        validate_revision(expected, "expected version")?;
        if content.trim().is_empty() {
            return Err(Error::Validation("draft content must not be empty".into()));
        }
        if !provenance.is_object() {
            return Err(Error::Validation(
                "draft provenance must be a JSON object".into(),
            ));
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let actual: Option<u64> = tx.query_row(
            "SELECT max(version) FROM draft_versions WHERE draft_id=?1",
            [id],
            |r| r.get(0),
        )?;
        let actual = actual.ok_or_else(|| Error::NotFound {
            kind: "draft",
            id: id.into(),
        })?;
        if actual != expected {
            return Err(Error::Stale {
                kind: "draft",
                id: id.into(),
                expected,
                actual,
            });
        }
        if actual == i64::MAX as u64 {
            return Err(Error::Conflict(format!(
                "draft `{id}` has reached the maximum version"
            )));
        }
        tx.execute("INSERT INTO draft_versions(draft_id,version,parent_version,content,author_kind,provenance_json) VALUES(?1,?2,?3,?4,?5,?6)",params![id,actual+1,actual,content,author_kind_db(author_kind),serde_json::to_string(provenance)?])?;
        tx.commit()?;
        self.get_draft(id, Some(actual + 1))
    }

    pub fn get_draft(&self, id: &str, version: Option<u64>) -> Result<DraftVersion> {
        validate_optional_revision(version, "draft version")?;
        get_draft_from(&self.conn, id, version)
    }

    pub fn list_draft_versions(&self, id: &str) -> Result<Vec<DraftVersion>> {
        let head = self.get_draft(id, None)?.version;
        (1..=head)
            .map(|version| self.get_draft(id, Some(version)))
            .collect()
    }

    pub fn attest(&mut self, input: &AttestationInput) -> Result<Vec<Attestation>> {
        if !input.human_approved {
            return Err(Error::Validation(
                "human_approved must be true; prose only records actual human approval".into(),
            ));
        }
        if input.approved_by.trim().is_empty() {
            return Err(Error::Validation("approved_by must not be empty".into()));
        }
        if input.via.harness.trim().is_empty() {
            return Err(Error::Validation("via.harness must not be empty".into()));
        }
        if input.via.session.trim().is_empty() {
            return Err(Error::Validation("via.session must not be empty".into()));
        }
        if input.items.is_empty() {
            return Err(Error::Validation("items must not be empty".into()));
        }
        let mut unique = std::collections::BTreeSet::new();
        if !input
            .items
            .iter()
            .any(|item| item.decision == AttestationDecision::Approved)
        {
            return Err(Error::Validation(
                "at least one attestation item must be approved".into(),
            ));
        }
        for item in &input.items {
            validate_revision(item.version, "draft version")?;
            if !super_key(&item.draft_id) {
                return Err(Error::Validation(
                    "draft id has invalid characters or length".into(),
                ));
            }
            for exception in &item.exceptions {
                if exception.trim().is_empty() {
                    return Err(Error::Validation(
                        "item exceptions must not be empty".into(),
                    ));
                }
            }
            if item.decision == AttestationDecision::Excepted && item.exceptions.is_empty() {
                return Err(Error::Validation(
                    "excepted items require an exception reason".into(),
                ));
            }
            if !unique.insert((&item.draft_id, item.version)) {
                return Err(Error::Validation(format!(
                    "duplicate draft target `{}@{}`",
                    item.draft_id, item.version
                )));
            }
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let batch_id = new_id(&tx)?;
        let tier_revision = resolve_ref(&tx, &input.audience_tier, RefKind::AudienceTier)?;
        for item in &input.items {
            get_draft_from(&tx, &item.draft_id, Some(item.version))?;
        }
        for item in &input.items {
            tx.execute("INSERT INTO attestations(id,batch_id,draft_id,draft_version,decision,exceptions_json) VALUES(lower(hex(randomblob(16))),?1,?2,?3,?4,?5)", params![batch_id, item.draft_id, item.version,attestation_decision_db(item.decision), serde_json::to_string(&item.exceptions)?])?;
        }
        tx.execute("INSERT INTO attestation_batches(id,human_approved,approved_by,via_harness,via_session,method,tier_pack_id,tier_pack_revision,tier_name) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9)",params![batch_id,input.human_approved,input.approved_by,input.via.harness,input.via.session,approval_method_db(input.method),input.audience_tier.pack_id,tier_revision,input.audience_tier.name])?;
        tx.commit()?;
        self.list_attestations(None, Some(&batch_id))
    }

    pub fn list_attestations(
        &self,
        draft: Option<&str>,
        batch: Option<&str>,
    ) -> Result<Vec<Attestation>> {
        let mut stmt=self.conn.prepare("SELECT a.id,a.batch_id,b.human_approved,b.approved_by,b.via_harness,b.via_session,b.method,b.tier_pack_id,b.tier_pack_revision,b.tier_name,a.draft_id,a.draft_version,a.decision,a.exceptions_json,b.created_at FROM attestations a JOIN attestation_batches b ON b.id=a.batch_id WHERE (?1 IS NULL OR a.draft_id=?1) AND (?2 IS NULL OR a.batch_id=?2) ORDER BY b.sequence,a.draft_id,a.draft_version,a.id")?;
        Ok(stmt
            .query_map(params![draft, batch], |r| {
                Ok(Attestation {
                    id: r.get(0)?,
                    batch_id: r.get(1)?,
                    human_approved: r.get(2)?,
                    approved_by: r.get(3)?,
                    via: ApprovalVia {
                        harness: r.get(4)?,
                        session: r.get(5)?,
                    },
                    method: approval_method_from_db(r.get::<_, String>(6)?.as_str())?,
                    audience_tier: RevisionRef {
                        pack_id: r.get(7)?,
                        pack_revision: r.get(8)?,
                        name: r.get(9)?,
                    },
                    draft: DraftTarget {
                        draft_id: r.get(10)?,
                        version: r.get(11)?,
                    },
                    decision: attestation_decision_from_db(r.get::<_, String>(12)?.as_str())?,
                    exceptions: json_column(r, 13)?,
                    created_at: r.get(14)?,
                })
            })?
            .collect::<rusqlite::Result<_>>()?)
    }

    pub fn record_capture(&mut self, input: &CaptureInput) -> Result<Capture> {
        if input.surface.trim().is_empty()
            || input.url.trim().is_empty()
            || input.content.trim().is_empty()
        {
            return Err(Error::Validation(
                "capture surface, url, and content must not be empty".into(),
            ));
        }
        if !input.metadata.is_object() {
            return Err(Error::Validation(
                "capture metadata must be a JSON object".into(),
            ));
        }
        let normalized_url = normalize_capture_url(&input.url)?;
        if let Some(draft) = &input.draft {
            validate_revision(draft.version, "draft version")?;
        }
        let tx = self
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(draft) = &input.draft {
            get_draft_from(&tx, &draft.draft_id, Some(draft.version))?;
        }
        let id = input.id.clone().unwrap_or(new_id(&tx)?);
        if !super_key(&id) {
            return Err(Error::Validation(
                "capture id has invalid characters or length".into(),
            ));
        }
        if let Some(existing) = tx
            .query_row(
                "SELECT id,observation,parent_id,surface,url,content,draft_id,draft_version,metadata_json,captured_at FROM capture_events WHERE id=?1",
                [&id],
                capture_row,
            )
            .optional()?
        {
            let same = existing.surface == input.surface
                && existing.url == input.url
                && existing.content == input.content
                && existing.draft == input.draft
                && existing.metadata == input.metadata
                && existing.observation == input.observation
                && existing.parent_id == input.parent_id;
            return if same {
                Ok(existing)
            } else {
                Err(Error::Conflict(format!(
                    "capture `{id}` already exists with different content"
                )))
            };
        }
        match input.observation {
            CaptureObservation::SubmitAttempt if input.parent_id.is_some() => {
                return Err(Error::Validation(
                    "submit-attempt capture must not have a parent".into(),
                ));
            }
            CaptureObservation::SurfaceConfirmedPost if input.parent_id.is_none() => {
                return Err(Error::Validation(
                    "surface-confirmed-post capture must reference a submit-attempt parent".into(),
                ));
            }
            _ => {}
        }
        if let Some(parent_id) = &input.parent_id {
            if !super_key(parent_id) {
                return Err(Error::Validation(
                    "capture parent id has invalid characters or length".into(),
                ));
            }
            let parent = tx
                .query_row(
                    "SELECT id,observation,parent_id,surface,url,content,draft_id,draft_version,metadata_json,captured_at FROM capture_events WHERE id=?1",
                    [parent_id],
                    capture_row,
                )
                .optional()?
                .ok_or_else(|| Error::NotFound {
                    kind: "capture",
                    id: parent_id.clone(),
                })?;
            if parent.observation != CaptureObservation::SubmitAttempt {
                return Err(Error::Validation(
                    "surface-confirmed-post parent must be a submit-attempt".into(),
                ));
            }
            if parent.surface != input.surface {
                return Err(Error::Validation(
                    "surface-confirmed-post must use its parent's surface".into(),
                ));
            }
            if normalize_capture_url(&parent.url)? != normalized_url {
                return Err(Error::Validation(
                    "surface-confirmed-post must use its parent's normalized URL".into(),
                ));
            }
        }
        tx.execute("INSERT INTO capture_events(id,observation,parent_id,normalized_url_key,surface,url,content,draft_id,draft_version,metadata_json) VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)", params![id,capture_observation_db(input.observation),input.parent_id,normalized_url,input.surface,input.url,input.content,input.draft.as_ref().map(|v|&v.draft_id),input.draft.as_ref().map(|v|v.version),serde_json::to_string(&input.metadata)?])?;
        tx.commit()?;
        self.get_capture(&id)
    }

    pub fn get_capture(&self, id: &str) -> Result<Capture> {
        self.conn.query_row("SELECT id,observation,parent_id,surface,url,content,draft_id,draft_version,metadata_json,captured_at FROM capture_events WHERE id=?1",[id],capture_row).optional()?.ok_or_else(||Error::NotFound{kind:"capture",id:id.into()})
    }

    pub fn list_captures(&self, surface: Option<&str>) -> Result<Vec<Capture>> {
        let mut stmt=self.conn.prepare("SELECT id,observation,parent_id,surface,url,content,draft_id,draft_version,metadata_json,captured_at FROM capture_events WHERE (?1 IS NULL OR surface=?1) ORDER BY sequence")?;
        Ok(stmt
            .query_map([surface], capture_row)?
            .collect::<rusqlite::Result<_>>()?)
    }

    fn serve_named_context(
        &self,
        name: &str,
        pack: Option<&str>,
    ) -> Result<(String, u64, ContextSpec)> {
        let mut stmt=self.conn.prepare("SELECT c.pack_id,c.pack_revision,c.name,c.description,c.content FROM context_revisions c JOIN (SELECT pack_id,max(revision) revision FROM pack_revisions GROUP BY pack_id) x ON x.pack_id=c.pack_id AND x.revision=c.pack_revision JOIN pack_revisions p ON p.pack_id=x.pack_id AND p.revision=x.revision WHERE p.tombstone=0 AND c.name=?1 AND (?2 IS NULL OR c.pack_id=?2) ORDER BY c.pack_id")?;
        let rows = stmt
            .query_map(params![name, pack], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    ContextSpec {
                        name: r.get(2)?,
                        description: r.get(3)?,
                        content: r.get(4)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        one_named(rows, "context", name)
    }
    fn serve_named_verb(&self, name: &str, pack: Option<&str>) -> Result<(String, u64, VerbSpec)> {
        let mut stmt=self.conn.prepare("SELECT c.pack_id,c.pack_revision,c.name,c.description,c.family,c.instructions,c.length_unit,c.length_min,c.length_max,c.no_new_claims,c.preserve_meaning,COALESCE((SELECT json_group_array(context_name) FROM (SELECT context_name FROM verb_context_binding_revisions b WHERE b.pack_id=c.pack_id AND b.pack_revision=c.pack_revision AND b.verb_name=c.name ORDER BY context_name)),'[]') FROM verb_revisions c JOIN (SELECT pack_id,max(revision) revision FROM pack_revisions GROUP BY pack_id) x ON x.pack_id=c.pack_id AND x.revision=c.pack_revision JOIN pack_revisions p ON p.pack_id=x.pack_id AND p.revision=x.revision WHERE p.tombstone=0 AND c.name=?1 AND (?2 IS NULL OR c.pack_id=?2) ORDER BY c.pack_id")?;
        let rows = stmt
            .query_map(params![name, pack], |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    VerbSpec {
                        name: r.get(2)?,
                        description: r.get(3)?,
                        family: verb_family_from_db(r.get::<_, String>(4)?.as_str())?,
                        instructions: r.get(5)?,
                        context_bindings: json_column(r, 11)?,
                        constraints: constraints_row(r, 6)?,
                    },
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        one_named(rows, "verb", name)
    }
}

fn active_pack_revision(conn: &Connection, id: &str) -> Result<u64> {
    let (revision, deleted) = latest_revision(conn, id)?.ok_or_else(|| Error::NotFound {
        kind: "pack",
        id: id.into(),
    })?;
    if deleted {
        return Err(Error::NotFound {
            kind: "pack",
            id: id.into(),
        });
    }
    Ok(revision)
}

fn selected_pack_from(conn: &Connection, explicit: Option<&str>) -> Result<String> {
    if let Some(id) = explicit {
        return Ok(id.into());
    }
    let value: Option<String> = conn
        .query_row(
            "SELECT value_json FROM setting_events WHERE key='active_pack' ORDER BY sequence DESC LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;
    value
        .map(|value| serde_json::from_str(&value).map_err(Error::Json))
        .transpose()?
        .ok_or_else(|| {
            Error::Conflict("no active pack; run `prose pack use <id>` or pass --pack".into())
        })
}

#[derive(Clone, Copy)]
enum RefKind {
    Context,
    Verb,
    AudienceTier,
}

fn resolve_ref(conn: &Connection, reference: &ContentRef, kind: RefKind) -> Result<u64> {
    if !super_key(&reference.pack_id) || !super_key(&reference.name) {
        return Err(Error::Validation(
            "content reference has invalid characters or length".into(),
        ));
    }
    let revision = active_pack_revision(conn, &reference.pack_id)?;
    let (table, label) = match kind {
        RefKind::Context => ("context_revisions", "context"),
        RefKind::Verb => ("verb_revisions", "verb"),
        RefKind::AudienceTier => ("audience_tier_revisions", "audience tier"),
    };
    let sql = format!(
        "SELECT EXISTS(SELECT 1 FROM {table} WHERE pack_id=?1 AND pack_revision=?2 AND name=?3)"
    );
    if !conn.query_row(
        &sql,
        params![reference.pack_id, revision, reference.name],
        |row| row.get::<_, bool>(0),
    )? {
        return Err(Error::NotFound {
            kind: label,
            id: format!("{}/{}", reference.pack_id, reference.name),
        });
    }
    Ok(revision)
}

fn get_draft_from(conn: &Connection, id: &str, version: Option<u64>) -> Result<DraftVersion> {
    let sql = if version.is_some() {
        "SELECT v.version,v.parent_version,v.content,v.created_at,d.context_pack_id,d.context_pack_revision,d.context_name,d.verb_pack_id,d.verb_pack_revision,d.verb_name,v.author_kind,v.provenance_json FROM draft_versions v JOIN drafts d ON d.id=v.draft_id WHERE v.draft_id=?1 AND v.version=?2"
    } else {
        "SELECT v.version,v.parent_version,v.content,v.created_at,d.context_pack_id,d.context_pack_revision,d.context_name,d.verb_pack_id,d.verb_pack_revision,d.verb_name,v.author_kind,v.provenance_json FROM draft_versions v JOIN drafts d ON d.id=v.draft_id WHERE v.draft_id=?1 ORDER BY v.version DESC LIMIT 1"
    };
    let mut stmt = conn.prepare(sql)?;
    let row = if let Some(value) = version {
        stmt.query_row(params![id, value], draft_row).optional()?
    } else {
        stmt.query_row([id], draft_row).optional()?
    };
    if let Some(mut value) = row {
        value.draft_id = id.into();
        return Ok(value);
    }
    let draft_exists = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM drafts WHERE id=?1)",
        [id],
        |row| row.get::<_, bool>(0),
    )?;
    if draft_exists {
        Err(Error::NotFound {
            kind: "draft version",
            id: version.map_or_else(|| id.into(), |value| format!("{id}@{value}")),
        })
    } else {
        Err(Error::NotFound {
            kind: "draft",
            id: id.into(),
        })
    }
}

fn validate_revision(value: u64, field: &str) -> Result<()> {
    if value == 0 || value > i64::MAX as u64 {
        return Err(Error::Validation(format!(
            "{field} must be between 1 and {}",
            i64::MAX
        )));
    }
    Ok(())
}

fn validate_optional_revision(value: Option<u64>, field: &str) -> Result<()> {
    if let Some(value) = value {
        validate_revision(value, field)?;
    }
    Ok(())
}

fn one_named<T>(
    mut rows: Vec<(String, u64, T)>,
    kind: &'static str,
    name: &str,
) -> Result<(String, u64, T)> {
    if rows.is_empty() {
        Err(Error::NotFound {
            kind,
            id: name.into(),
        })
    } else if rows.len() > 1 {
        Err(Error::Conflict(format!(
            "{kind} `{name}` is ambiguous; specify --pack"
        )))
    } else {
        Ok(rows.remove(0))
    }
}
fn latest_revision(conn: &Connection, id: &str) -> Result<Option<(u64, bool)>> {
    Ok(conn.query_row("SELECT revision,tombstone FROM pack_revisions WHERE pack_id=?1 ORDER BY revision DESC LIMIT 1",[id],|r|Ok((r.get(0)?,r.get(1)?))).optional()?)
}
fn insert_pack_revision(
    tx: &rusqlite::Transaction<'_>,
    doc: &PackDocument,
    revision: u64,
    tombstone: bool,
) -> Result<()> {
    tx.execute(
        "INSERT INTO pack_revisions(pack_id,revision,description,tombstone) VALUES(?1,?2,?3,?4)",
        params![doc.id, revision, doc.description, tombstone],
    )?;
    for v in &doc.contexts {
        tx.execute(
            "INSERT INTO context_revisions VALUES(?1,?2,?3,?4,?5)",
            params![doc.id, revision, v.name, v.description, v.content],
        )?;
    }
    for v in &doc.verbs {
        let (length_unit, length_min, length_max) = match &v.constraints.length {
            Some(length) => (Some(length_unit_db(length.unit)), length.min, length.max),
            None => (None, None, None),
        };
        tx.execute(
            "INSERT INTO verb_revisions VALUES(?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
            params![
                doc.id,
                revision,
                v.name,
                v.description,
                verb_family_db(v.family),
                v.instructions,
                length_unit,
                length_min,
                length_max,
                v.constraints.no_new_claims,
                v.constraints.preserve_meaning
            ],
        )?;
        for context_name in &v.context_bindings {
            tx.execute(
                "INSERT INTO verb_context_binding_revisions VALUES(?1,?2,?3,?4)",
                params![doc.id, revision, v.name, context_name],
            )?;
        }
    }
    for v in &doc.audience_tiers {
        tx.execute(
            "INSERT INTO audience_tier_revisions VALUES(?1,?2,?3,?4,?5)",
            params![
                doc.id,
                revision,
                v.name,
                v.description,
                serde_json::to_string(&v.requirements)?
            ],
        )?;
    }
    for v in &doc.surface_mappings {
        tx.execute(
            "INSERT INTO surface_mapping_revisions VALUES(?1,?2,?3,?4,?5)",
            params![
                doc.id,
                revision,
                v.surface,
                v.context,
                v.default_audience_tier
            ],
        )?;
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum NewOrigin<'a> {
    Approved {
        attestation_id: &'a str,
        draft_id: &'a str,
        draft_version: u64,
    },
    External {
        source_label: Option<&'a str>,
        source_uri: Option<&'a str>,
        channel: &'a str,
    },
}

fn origin_link_table(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Context => "context_revision_origins",
        ItemKind::Verb => "verb_revision_origins",
        ItemKind::Tier => "audience_tier_revision_origins",
        ItemKind::Surface => "surface_mapping_revision_origins",
    }
}

fn origin_key_column(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Surface => "surface",
        _ => "name",
    }
}

fn populate_revision_origins(
    tx: &rusqlite::Transaction<'_>,
    doc: &PackDocument,
    revision: u64,
    previous: Option<(u64, &PackDocument)>,
    channel: &str,
    override_origin: Option<(ItemKind, &str, NewOrigin<'_>)>,
) -> Result<()> {
    validate_origin_channel(channel)?;
    for item in &doc.contexts {
        let unchanged = previous.is_some_and(|(_, old)| old.contexts.iter().any(|v| v == item));
        ensure_item_origin(
            tx,
            &doc.id,
            revision,
            previous.map(|v| v.0),
            ItemKind::Context,
            &item.name,
            unchanged,
            &serde_json::to_value(item)?,
            channel,
            override_origin,
        )?;
    }
    for item in &doc.verbs {
        let unchanged = previous.is_some_and(|(_, old)| old.verbs.iter().any(|v| v == item));
        ensure_item_origin(
            tx,
            &doc.id,
            revision,
            previous.map(|v| v.0),
            ItemKind::Verb,
            &item.name,
            unchanged,
            &serde_json::to_value(item)?,
            channel,
            override_origin,
        )?;
    }
    for item in &doc.audience_tiers {
        let unchanged =
            previous.is_some_and(|(_, old)| old.audience_tiers.iter().any(|v| v == item));
        ensure_item_origin(
            tx,
            &doc.id,
            revision,
            previous.map(|v| v.0),
            ItemKind::Tier,
            &item.name,
            unchanged,
            &serde_json::to_value(item)?,
            channel,
            override_origin,
        )?;
    }
    for item in &doc.surface_mappings {
        let unchanged =
            previous.is_some_and(|(_, old)| old.surface_mappings.iter().any(|v| v == item));
        ensure_item_origin(
            tx,
            &doc.id,
            revision,
            previous.map(|v| v.0),
            ItemKind::Surface,
            &item.surface,
            unchanged,
            &serde_json::to_value(item)?,
            channel,
            override_origin,
        )?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn ensure_item_origin(
    tx: &rusqlite::Transaction<'_>,
    pack_id: &str,
    revision: u64,
    previous_revision: Option<u64>,
    kind: ItemKind,
    key: &str,
    unchanged: bool,
    material: &serde_json::Value,
    channel: &str,
    override_origin: Option<(ItemKind, &str, NewOrigin<'_>)>,
) -> Result<()> {
    let explicit = override_origin
        .filter(|(override_kind, override_key, _)| *override_kind == kind && *override_key == key)
        .map(|(_, _, origin)| origin);
    if explicit.is_none()
        && unchanged
        && let Some(origin_id) = previous_revision
            .map(|old| existing_origin_id(tx, pack_id, old, kind, key))
            .transpose()?
            .flatten()
    {
        return link_origin(tx, pack_id, revision, kind, key, origin_id);
    }
    let origin = explicit.unwrap_or(NewOrigin::External {
        source_label: None,
        source_uri: None,
        channel,
    });
    let origin_id = insert_origin(tx, origin, kind, material)?;
    link_origin(tx, pack_id, revision, kind, key, origin_id)
}

fn existing_origin_id(
    conn: &Connection,
    pack_id: &str,
    revision: u64,
    kind: ItemKind,
    key: &str,
) -> Result<Option<i64>> {
    let table = origin_link_table(kind);
    let key_column = origin_key_column(kind);
    let sql = format!(
        "SELECT origin_id FROM {table} WHERE pack_id=?1 AND pack_revision=?2 AND {key_column}=?3"
    );
    Ok(conn
        .query_row(&sql, params![pack_id, revision, key], |row| row.get(0))
        .optional()?)
}

fn insert_origin(
    tx: &rusqlite::Transaction<'_>,
    origin: NewOrigin<'_>,
    item_kind: ItemKind,
    material: &serde_json::Value,
) -> Result<i64> {
    let kind = match origin {
        NewOrigin::Approved { .. } => "approved-draft",
        NewOrigin::External { .. } => "external-import",
    };
    tx.execute("INSERT INTO pack_item_origins(kind) VALUES(?1)", [kind])?;
    let id = tx.last_insert_rowid();
    match origin {
        NewOrigin::Approved {
            attestation_id,
            draft_id,
            draft_version,
        } => {
            tx.execute("INSERT INTO approved_draft_origins(origin_id,attestation_id,draft_id,draft_version) VALUES(?1,?2,?3,?4)", params![id,attestation_id,draft_id,draft_version])?;
        }
        NewOrigin::External {
            source_label,
            source_uri,
            channel,
        } => {
            let digest = canonical_material_digest(item_kind, material)?;
            tx.execute("INSERT INTO external_import_origins(origin_id,source_label,source_uri,material_sha256,channel) VALUES(?1,?2,?3,?4,?5)", params![id,source_label,source_uri,digest,channel])?;
        }
    }
    Ok(id)
}

fn link_origin(
    tx: &rusqlite::Transaction<'_>,
    pack_id: &str,
    revision: u64,
    kind: ItemKind,
    key: &str,
    origin_id: i64,
) -> Result<()> {
    let table = origin_link_table(kind);
    let key_column = origin_key_column(kind);
    let sql = format!(
        "INSERT INTO {table}(pack_id,pack_revision,{key_column},origin_id) VALUES(?1,?2,?3,?4)"
    );
    tx.execute(&sql, params![pack_id, revision, key, origin_id])?;
    Ok(())
}

fn canonical_material_digest(kind: ItemKind, value: &serde_json::Value) -> Result<String> {
    fn canonical(value: &serde_json::Value, out: &mut String) -> Result<()> {
        match value {
            serde_json::Value::Object(map) => {
                out.push('{');
                let mut keys = map.keys().collect::<Vec<_>>();
                keys.sort();
                for (index, key) in keys.into_iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    out.push_str(&serde_json::to_string(key)?);
                    out.push(':');
                    canonical(&map[key], out)?;
                }
                out.push('}');
            }
            serde_json::Value::Array(items) => {
                out.push('[');
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push(',');
                    }
                    canonical(item, out)?;
                }
                out.push(']');
            }
            other => out.push_str(&serde_json::to_string(other)?),
        }
        Ok(())
    }
    let mut encoded = format!("prose:pack-item:v1:{}\0", item_kind_domain(kind));
    canonical(value, &mut encoded)?;
    Ok(format!("{:x}", Sha256::digest(encoded.as_bytes())))
}

fn item_kind_domain(kind: ItemKind) -> &'static str {
    match kind {
        ItemKind::Context => "context",
        ItemKind::Verb => "verb",
        ItemKind::Tier => "tier",
        ItemKind::Surface => "surface",
    }
}

fn validate_origin_channel(channel: &str) -> Result<()> {
    if channel.trim().is_empty() {
        Err(Error::Validation("origin channel must not be empty".into()))
    } else {
        Ok(())
    }
}

fn origin_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<PackItemOrigin> {
    match row.get::<_, String>(0)?.as_str() {
        "approved-draft" => Ok(PackItemOrigin::ApprovedDraft {
            attestation_id: row.get(2)?,
            draft_id: row.get(3)?,
            draft_version: row.get(4)?,
        }),
        "external-import" => Ok(PackItemOrigin::ExternalImport {
            source_label: row.get(5)?,
            source_uri: row.get(6)?,
            material_sha256: row.get(7)?,
            channel: row.get(8)?,
            recorded_at: row.get(1)?,
        }),
        _ => Err(conversion_error(0, "invalid stored pack item origin kind")),
    }
}

fn canonical_pack(doc: &PackDocument) -> PackDocument {
    let mut doc = doc.clone();
    doc.contexts.sort_by(|a, b| a.name.cmp(&b.name));
    doc.verbs.sort_by(|a, b| a.name.cmp(&b.name));
    for verb in &mut doc.verbs {
        verb.context_bindings.sort();
    }
    doc.audience_tiers.sort_by(|a, b| a.name.cmp(&b.name));
    doc.surface_mappings
        .sort_by(|a, b| a.surface.cmp(&b.surface));
    doc
}
fn load_pack(conn: &Connection, id: &str, revision: u64, deleted: bool) -> Result<Pack> {
    let description: String = conn.query_row(
        "SELECT description FROM pack_revisions WHERE pack_id=?1 AND revision=?2",
        params![id, revision],
        |r| r.get(0),
    )?;
    let contexts = collect(
        conn,
        "SELECT name,description,content FROM context_revisions WHERE pack_id=?1 AND pack_revision=?2 ORDER BY name",
        id,
        revision,
        |r| {
            Ok(ContextSpec {
                name: r.get(0)?,
                description: r.get(1)?,
                content: r.get(2)?,
            })
        },
    )?;
    let verbs = collect(
        conn,
        "SELECT name,description,family,instructions,length_unit,length_min,length_max,no_new_claims,preserve_meaning,COALESCE((SELECT json_group_array(context_name) FROM (SELECT context_name FROM verb_context_binding_revisions b WHERE b.pack_id=verb_revisions.pack_id AND b.pack_revision=verb_revisions.pack_revision AND b.verb_name=verb_revisions.name ORDER BY context_name)),'[]') FROM verb_revisions WHERE pack_id=?1 AND pack_revision=?2 ORDER BY name",
        id,
        revision,
        |r| {
            Ok(VerbSpec {
                name: r.get(0)?,
                description: r.get(1)?,
                family: verb_family_from_db(r.get::<_, String>(2)?.as_str())?,
                instructions: r.get(3)?,
                context_bindings: json_column(r, 9)?,
                constraints: constraints_row(r, 4)?,
            })
        },
    )?;
    let audience_tiers = collect(
        conn,
        "SELECT name,description,requirements_json FROM audience_tier_revisions WHERE pack_id=?1 AND pack_revision=?2 ORDER BY name",
        id,
        revision,
        |r| {
            let json: String = r.get(2)?;
            let requirements = serde_json::from_str(&json).map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    2,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })?;
            Ok(AudienceTier {
                name: r.get(0)?,
                description: r.get(1)?,
                requirements,
            })
        },
    )?;
    let surface_mappings = collect(
        conn,
        "SELECT surface,context_name,default_audience_tier FROM surface_mapping_revisions WHERE pack_id=?1 AND pack_revision=?2 ORDER BY surface",
        id,
        revision,
        |r| {
            Ok(SurfaceMapping {
                surface: r.get(0)?,
                context: r.get(1)?,
                default_audience_tier: r.get(2)?,
            })
        },
    )?;
    Ok(Pack {
        revision,
        deleted,
        document: PackDocument {
            id: id.into(),
            description,
            contexts,
            verbs,
            audience_tiers,
            surface_mappings,
        },
    })
}
fn collect<T, F>(conn: &Connection, sql: &str, id: &str, revision: u64, mut f: F) -> Result<Vec<T>>
where
    F: FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<T>,
{
    let mut stmt = conn.prepare(sql)?;
    Ok(stmt
        .query_map(params![id, revision], |r| f(r))?
        .collect::<rusqlite::Result<_>>()?)
}
fn draft_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<DraftVersion> {
    let cp: Option<String> = r.get(4)?;
    let cr: Option<u64> = r.get(5)?;
    let cn: Option<String> = r.get(6)?;
    let vp: Option<String> = r.get(7)?;
    let vr: Option<u64> = r.get(8)?;
    let vn: Option<String> = r.get(9)?;
    Ok(DraftVersion {
        draft_id: String::new(),
        version: r.get(0)?,
        parent_version: r.get(1)?,
        content: r.get(2)?,
        created_at: r.get(3)?,
        context: cp
            .zip(cr)
            .zip(cn)
            .map(|((pack_id, pack_revision), name)| RevisionRef {
                pack_id,
                pack_revision,
                name,
            }),
        verb: vp
            .zip(vr)
            .zip(vn)
            .map(|((pack_id, pack_revision), name)| RevisionRef {
                pack_id,
                pack_revision,
                name,
            }),
        author_kind: author_kind_from_db(r.get::<_, String>(10)?.as_str())?,
        provenance: json_column(r, 11)?,
    })
}

fn capture_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Capture> {
    let draft_id: Option<String> = r.get(6)?;
    let draft_version: Option<u64> = r.get(7)?;
    Ok(Capture {
        id: r.get(0)?,
        observation: capture_observation_from_db(r.get::<_, String>(1)?.as_str())?,
        parent_id: r.get(2)?,
        surface: r.get(3)?,
        url: r.get(4)?,
        content: r.get(5)?,
        draft: draft_id
            .zip(draft_version)
            .map(|(draft_id, version)| DraftTarget { draft_id, version }),
        metadata: json_column(r, 8)?,
        captured_at: r.get(9)?,
    })
}

fn capture_observation_db(value: CaptureObservation) -> &'static str {
    match value {
        CaptureObservation::SubmitAttempt => "submit-attempt",
        CaptureObservation::SurfaceConfirmedPost => "surface-confirmed-post",
    }
}

fn capture_observation_from_db(value: &str) -> rusqlite::Result<CaptureObservation> {
    match value {
        "submit-attempt" => Ok(CaptureObservation::SubmitAttempt),
        "surface-confirmed-post" => Ok(CaptureObservation::SurfaceConfirmedPost),
        _ => Err(conversion_error(1, "invalid stored capture observation")),
    }
}

fn normalize_capture_url(value: &str) -> Result<String> {
    let mut url = Url::parse(value)
        .map_err(|_| Error::Validation("capture URL must be an absolute URL".into()))?;
    url.set_query(None);
    url.set_fragment(None);
    Ok(url.to_string())
}

fn apply_schema_v3(tx: &rusqlite::Transaction<'_>) -> Result<()> {
    tx.execute_batch(SCHEMA_V3_PRE)?;
    let rows = {
        let mut statement = tx.prepare("SELECT sequence,url FROM capture_events")?;
        statement
            .query_map([], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (sequence, url) in rows {
        // Schema v2 accepted any non-empty URL string. Preserve those legacy
        // submit attempts even when they predate v3's absolute-URL contract;
        // new events still pass through `normalize_capture_url` strictly.
        let normalized = normalize_capture_url(&url).unwrap_or_else(|_| format!("legacy:{url}"));
        tx.execute(
            "UPDATE capture_events SET normalized_url_key=?1 WHERE sequence=?2",
            params![normalized, sequence],
        )?;
    }
    tx.execute_batch(SCHEMA_V3)?;
    Ok(())
}

fn author_kind_db(value: AuthorKind) -> &'static str {
    match value {
        AuthorKind::Human => "human",
        AuthorKind::Agent => "agent",
        AuthorKind::Capture => "capture",
        AuthorKind::Import => "import",
    }
}
fn author_kind_from_db(value: &str) -> rusqlite::Result<AuthorKind> {
    match value {
        "human" => Ok(AuthorKind::Human),
        "agent" => Ok(AuthorKind::Agent),
        "capture" => Ok(AuthorKind::Capture),
        "import" => Ok(AuthorKind::Import),
        _ => Err(conversion_error(10, "invalid stored author kind")),
    }
}
fn attestation_decision_db(value: AttestationDecision) -> &'static str {
    match value {
        AttestationDecision::Approved => "approved",
        AttestationDecision::Excepted => "excepted",
    }
}
fn attestation_decision_from_db(value: &str) -> rusqlite::Result<AttestationDecision> {
    match value {
        "approved" => Ok(AttestationDecision::Approved),
        "excepted" => Ok(AttestationDecision::Excepted),
        _ => Err(conversion_error(12, "invalid stored attestation decision")),
    }
}

fn conversion_error(index: usize, message: impl Into<String>) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        index,
        rusqlite::types::Type::Text,
        Box::new(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            message.into(),
        )),
    )
}

fn json_column<T: serde::de::DeserializeOwned>(
    row: &rusqlite::Row<'_>,
    index: usize,
) -> rusqlite::Result<T> {
    let value: String = row.get(index)?;
    serde_json::from_str(&value).map_err(|error| {
        rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(error),
        )
    })
}

fn verb_family_db(value: VerbFamily) -> &'static str {
    match value {
        VerbFamily::TransformSelection => "transform-selection",
        VerbFamily::DraftFromIntent => "draft-from-intent",
    }
}

fn verb_family_from_db(value: &str) -> rusqlite::Result<VerbFamily> {
    match value {
        "transform-selection" => Ok(VerbFamily::TransformSelection),
        "draft-from-intent" => Ok(VerbFamily::DraftFromIntent),
        _ => Err(conversion_error(0, "invalid stored verb family")),
    }
}

fn length_unit_db(value: LengthUnit) -> &'static str {
    match value {
        LengthUnit::Characters => "characters",
        LengthUnit::Words => "words",
    }
}

fn length_unit_from_db(value: &str) -> rusqlite::Result<LengthUnit> {
    match value {
        "characters" => Ok(LengthUnit::Characters),
        "words" => Ok(LengthUnit::Words),
        _ => Err(conversion_error(0, "invalid stored length unit")),
    }
}

fn constraints_row(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<VerbConstraints> {
    let unit: Option<String> = row.get(start)?;
    let min = row.get(start + 1)?;
    let max = row.get(start + 2)?;
    let length = unit
        .as_deref()
        .map(length_unit_from_db)
        .transpose()?
        .map(|unit| LengthConstraint { unit, min, max });
    Ok(VerbConstraints {
        length,
        no_new_claims: row.get(start + 3)?,
        preserve_meaning: row.get(start + 4)?,
    })
}

fn approval_method_db(value: ApprovalMethod) -> &'static str {
    match value {
        ApprovalMethod::VerbalLgtm => "verbal-lgtm",
        ApprovalMethod::Explicit => "explicit",
    }
}

fn approval_method_from_db(value: &str) -> rusqlite::Result<ApprovalMethod> {
    match value {
        "verbal-lgtm" => Ok(ApprovalMethod::VerbalLgtm),
        "explicit" => Ok(ApprovalMethod::Explicit),
        _ => Err(conversion_error(0, "invalid stored approval method")),
    }
}

fn new_id(conn: &Connection) -> Result<String> {
    Ok(conn.query_row("SELECT lower(hex(randomblob(16)))", [], |row| row.get(0))?)
}
fn super_key(v: &str) -> bool {
    !v.is_empty()
        && v.len() <= 128
        && v.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn verify_schema(conn: &Connection) -> Result<()> {
    let application_id: u32 = conn.pragma_query_value(None, "application_id", |row| row.get(0))?;
    if application_id != APPLICATION_ID {
        return Err(Error::Conflict(
            "store schema identity is missing or unrecognized".into(),
        ));
    }
    for name in [
        "schema_migrations",
        "packs",
        "setting_events",
        "pack_revisions",
        "context_revisions",
        "verb_revisions",
        "verb_context_binding_revisions",
        "audience_tier_revisions",
        "surface_mapping_revisions",
        "drafts",
        "draft_versions",
        "attestation_batches",
        "attestations",
        "capture_events",
        "pack_item_origins",
        "approved_draft_origins",
        "external_import_origins",
        "context_revision_origins",
        "verb_revision_origins",
        "audience_tier_revision_origins",
        "surface_mapping_revision_origins",
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='table' AND name=?1)",
            [name],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::Conflict(format!(
                "store schema is missing table `{name}`"
            )));
        }
        let strict: bool = conn.query_row(
            "SELECT strict FROM pragma_table_list WHERE name=?1 AND schema='main'",
            [name],
            |row| row.get(0),
        )?;
        if !strict {
            return Err(Error::Conflict(format!(
                "store schema table `{name}` is not STRICT"
            )));
        }
    }
    for (table, columns) in [
        ("schema_migrations", &["version", "applied_at"][..]),
        ("packs", &["id"][..]),
        (
            "setting_events",
            &["sequence", "key", "value_json", "created_at"],
        ),
        (
            "pack_revisions",
            &[
                "pack_id",
                "revision",
                "description",
                "tombstone",
                "created_at",
            ],
        ),
        (
            "context_revisions",
            &["pack_id", "pack_revision", "name", "description", "content"],
        ),
        (
            "verb_revisions",
            &[
                "pack_id",
                "pack_revision",
                "name",
                "description",
                "family",
                "instructions",
                "length_unit",
                "length_min",
                "length_max",
                "no_new_claims",
                "preserve_meaning",
            ],
        ),
        (
            "verb_context_binding_revisions",
            &["pack_id", "pack_revision", "verb_name", "context_name"],
        ),
        (
            "audience_tier_revisions",
            &[
                "pack_id",
                "pack_revision",
                "name",
                "description",
                "requirements_json",
            ],
        ),
        (
            "surface_mapping_revisions",
            &[
                "pack_id",
                "pack_revision",
                "surface",
                "context_name",
                "default_audience_tier",
            ],
        ),
        (
            "drafts",
            &[
                "id",
                "context_pack_id",
                "context_pack_revision",
                "context_name",
                "verb_pack_id",
                "verb_pack_revision",
                "verb_name",
                "created_at",
            ],
        ),
        (
            "draft_versions",
            &[
                "draft_id",
                "version",
                "parent_version",
                "content",
                "author_kind",
                "provenance_json",
                "created_at",
            ],
        ),
        (
            "attestation_batches",
            &[
                "sequence",
                "id",
                "human_approved",
                "approved_by",
                "via_harness",
                "via_session",
                "method",
                "tier_pack_id",
                "tier_pack_revision",
                "tier_name",
                "created_at",
            ],
        ),
        (
            "attestations",
            &[
                "id",
                "batch_id",
                "draft_id",
                "draft_version",
                "decision",
                "exceptions_json",
            ],
        ),
        (
            "capture_events",
            &[
                "sequence",
                "id",
                "surface",
                "url",
                "content",
                "draft_id",
                "draft_version",
                "metadata_json",
                "captured_at",
                "observation",
                "parent_id",
                "normalized_url_key",
            ],
        ),
        ("pack_item_origins", &["id", "kind", "recorded_at"]),
        (
            "approved_draft_origins",
            &["origin_id", "attestation_id", "draft_id", "draft_version"],
        ),
        (
            "external_import_origins",
            &[
                "origin_id",
                "source_label",
                "source_uri",
                "material_sha256",
                "channel",
            ],
        ),
        (
            "context_revision_origins",
            &["pack_id", "pack_revision", "name", "origin_id"],
        ),
        (
            "verb_revision_origins",
            &["pack_id", "pack_revision", "name", "origin_id"],
        ),
        (
            "audience_tier_revision_origins",
            &["pack_id", "pack_revision", "name", "origin_id"],
        ),
        (
            "surface_mapping_revision_origins",
            &["pack_id", "pack_revision", "surface", "origin_id"],
        ),
    ] {
        let mut statement = conn.prepare("SELECT name FROM pragma_table_xinfo(?1) ORDER BY cid")?;
        let actual = statement
            .query_map([table], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        if actual.iter().map(String::as_str).collect::<Vec<_>>() != columns {
            return Err(Error::Conflict(format!(
                "store schema table `{table}` has unexpected columns"
            )));
        }
    }
    for name in [
        "packs_no_update",
        "packs_no_delete",
        "settings_no_update",
        "settings_no_delete",
        "pack_revisions_no_update",
        "pack_revisions_no_delete",
        "context_revisions_no_update",
        "context_revisions_no_delete",
        "verb_revisions_no_update",
        "verb_revisions_no_delete",
        "verb_bindings_no_update",
        "verb_bindings_no_delete",
        "tier_revisions_no_update",
        "tier_revisions_no_delete",
        "tier_requirements_typed",
        "mappings_no_update",
        "mappings_no_delete",
        "draft_versions_no_update",
        "draft_versions_no_delete",
        "drafts_no_update",
        "drafts_no_delete",
        "attestations_no_update",
        "attestations_no_delete",
        "attestation_exceptions_typed",
        "attestation_batch_closed",
        "batches_no_update",
        "batches_no_delete",
        "captures_no_update",
        "captures_no_delete",
        "captures_normalized_url_required",
        "capture_confirmation_parent_distinct",
        "capture_confirmation_parent_exists",
        "capture_confirmation_parent_attempt",
        "capture_confirmation_parent_target",
        "approved_origin_requires_approval",
        "approved_origin_kind",
        "external_origin_kind",
        "context_origin_complete",
        "context_approved_material",
        "verb_origin_complete",
        "verb_approved_material",
        "tier_origin_complete",
        "tier_rejects_approved_origin",
        "surface_origin_complete",
        "surface_rejects_approved_origin",
        "origins_no_update",
        "origins_no_delete",
        "approved_origins_no_update",
        "approved_origins_no_delete",
        "external_origins_no_update",
        "external_origins_no_delete",
        "context_origins_no_update",
        "context_origins_no_delete",
        "verb_origins_no_update",
        "verb_origins_no_delete",
        "tier_origins_no_update",
        "tier_origins_no_delete",
        "surface_origins_no_update",
        "surface_origins_no_delete",
    ] {
        let exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_schema WHERE type='trigger' AND name=?1)",
            [name],
            |row| row.get(0),
        )?;
        if !exists {
            return Err(Error::Conflict(format!(
                "store schema is missing trigger `{name}`"
            )));
        }
    }
    let foreign_key_errors: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM pragma_foreign_key_check)",
        [],
        |row| row.get(0),
    )?;
    if foreign_key_errors {
        return Err(Error::Conflict(
            "store contains foreign-key violations".into(),
        ));
    }
    Ok(())
}

fn prepare_store_dir(path: &Path) -> std::io::Result<()> {
    if path == Path::new(".") {
        return Ok(());
    }
    let mut missing = Vec::new();
    let mut cursor = path;
    while !cursor.exists() {
        missing.push(cursor.to_path_buf());
        cursor = cursor.parent().unwrap_or_else(|| Path::new("."));
    }
    let metadata = fs::symlink_metadata(cursor)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "store directory ancestor must be a real directory",
        ));
    }
    for directory in missing.iter().rev() {
        match fs::create_dir(directory) {
            Ok(()) => private_dir(directory)?,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                let metadata = fs::symlink_metadata(directory)?;
                if metadata.file_type().is_symlink() || !metadata.is_dir() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidInput,
                        "store directory must be a real directory",
                    ));
                }
            }
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn prepare_store_file(path: &Path) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    if path.exists() {
        return validate_private_file(path, "store path");
    }
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    set_private_create_mode(&mut options);
    match options.open(path) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            validate_private_file(path, "store path")
        }
        Err(error) => Err(error),
    }
}

fn private_sqlite_sidecars(path: &Path) -> std::io::Result<()> {
    for suffix in ["-wal", "-shm"] {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        let sidecar = PathBuf::from(name);
        if fs::symlink_metadata(&sidecar).is_ok() {
            validate_private_file(&sidecar, "SQLite sidecar")?;
        }
    }
    Ok(())
}

fn validate_private_file(path: &Path, label: &str) -> std::io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{label} must be a regular file"),
        ));
    }
    require_single_link(&metadata)?;
    private_file(path)
}

#[cfg(unix)]
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}
#[cfg(unix)]
fn set_private_create_mode(options: &mut std::fs::OpenOptions) {
    use std::os::unix::fs::OpenOptionsExt;
    options.mode(0o600);
}
#[cfg(not(unix))]
fn set_private_create_mode(_: &mut std::fs::OpenOptions) {}
#[cfg(unix)]
fn require_single_link(metadata: &fs::Metadata) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    if metadata.nlink() != 1 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "store file must not have multiple hard links",
        ));
    }
    Ok(())
}
#[cfg(not(unix))]
fn require_single_link(_: &fs::Metadata) -> std::io::Result<()> {
    Ok(())
}
#[cfg(not(unix))]
fn private_dir(_: &Path) -> std::io::Result<()> {
    Ok(())
}
#[cfg(unix)]
fn private_file(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(path, fs::Permissions::from_mode(0o600))
}
#[cfg(not(unix))]
fn private_file(_: &Path) -> std::io::Result<()> {
    Ok(())
}
