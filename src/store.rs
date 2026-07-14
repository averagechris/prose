use crate::model::*;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

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
const APPLICATION_ID: u32 = 0x5052_4f53;

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
        if version > 1 {
            return Err(Error::Conflict(format!(
                "store schema version {version} is newer than supported version 1"
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
            tx.pragma_update(None, "application_id", APPLICATION_ID)?;
            tx.pragma_update(None, "user_version", 1)?;
            tx.commit()?;
        } else {
            verify_schema(&conn)?;
        }
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
        tx.commit()?;
        Ok(Pack {
            revision: 1,
            deleted: false,
            document: doc,
        })
    }

    pub fn update_pack(&mut self, doc: &PackDocument, expected: u64) -> Result<Pack> {
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
        let revision = actual + 1;
        insert_pack_revision(&tx, &doc, revision, false)?;
        tx.commit()?;
        Ok(Pack {
            revision,
            deleted: false,
            document: doc,
        })
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
        if let Some(id) = explicit {
            return Ok(id.into());
        }
        self.active_pack()?.ok_or_else(|| {
            Error::Conflict("no active pack; run `prose pack use <id>` or pass --pack".into())
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
