//! Transport-neutral prose operations shared by the CLI and MCP server.

use crate::{
    AssistPreparation, AttestationInput, AudienceTier, AuthorKind, CaptureInput, ContextSpec,
    DraftCreate, DraftTarget, DraftVersion, Error, ItemKind, PackDocument, PreparedAssist, Result,
    Store, SurfaceMapping, VerbSpec,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct StoreReadiness {
    pub ready: bool,
    pub category: Option<&'static str>,
}

#[derive(Debug, Clone)]
pub struct Application {
    store_path: PathBuf,
}

impl Application {
    pub fn new(store_path: impl Into<PathBuf>) -> Self {
        Self {
            store_path: store_path.into(),
        }
    }

    pub fn store_path(&self) -> &Path {
        &self.store_path
    }

    /// Opens and queries the store rather than relying on construction-time state.
    pub fn store_readiness(&self) -> StoreReadiness {
        match Store::open(&self.store_path).and_then(|store| store.schema_version()) {
            Ok(_) => StoreReadiness {
                ready: true,
                category: None,
            },
            Err(error) => StoreReadiness {
                ready: false,
                category: Some(match error {
                    Error::Sql(_) => "storage-error",
                    Error::Io(_) => "io-error",
                    Error::Json(_) => "data-error",
                    Error::Validation(_)
                    | Error::NotFound { .. }
                    | Error::Stale { .. }
                    | Error::Conflict(_) => "store-error",
                }),
            },
        }
    }

    pub fn pack(&self, request: PackRequest) -> Result<Value> {
        let mut store = Store::open(&self.store_path)?;
        match request {
            PackRequest::Create { document } => mutation("created", store.create_pack(&document)?),
            PackRequest::Update {
                document,
                expected_revision,
            } => mutation("updated", store.update_pack(&document, expected_revision)?),
            PackRequest::Get { id, revision } => to_value(store.get_pack(&id, revision)?),
            PackRequest::List => Ok(json!({"items":store.list_packs()?})),
            PackRequest::Delete {
                id,
                expected_revision,
            } => mutation("deleted", store.delete_pack(&id, expected_revision)?),
            PackRequest::Use { id } => mutation("selected", store.use_pack(&id)?),
            PackRequest::Import {
                document,
                expected_revision,
                activate,
            } => {
                let pack = match expected_revision {
                    Some(revision) => {
                        store.update_pack_external(&document, revision, "pack-import")?
                    }
                    None => store.create_pack_external(&document, "pack-import")?,
                };
                if activate {
                    store.use_pack(&document.id)?;
                }
                mutation("imported", pack)
            }
            PackRequest::Export { id, revision } => {
                to_value(store.get_pack(&id, revision)?.document)
            }
            PackRequest::ItemPut {
                pack_id,
                kind,
                id,
                document,
                expected_revision,
            } => revise_external_item(
                &mut store,
                ExternalItemRevision {
                    pack_id: &pack_id,
                    kind,
                    id: &id,
                    document: Some(document),
                    expected: expected_revision,
                    source_label: None,
                    source_uri: None,
                    channel: "item-put",
                },
            ),
            PackRequest::ItemImport {
                pack_id,
                kind,
                id,
                document,
                expected_revision,
                source_label,
                source_uri,
                channel,
            } => revise_external_item(
                &mut store,
                ExternalItemRevision {
                    pack_id: &pack_id,
                    kind,
                    id: &id,
                    document: Some(document),
                    expected: expected_revision,
                    source_label: source_label.as_deref(),
                    source_uri: source_uri.as_deref(),
                    channel: &channel,
                },
            ),
            PackRequest::ItemInstallApprovedDraft {
                pack_id,
                kind,
                id,
                document,
                expected_revision,
                attestation_id,
                source,
            } => install_approved_item(
                &mut store,
                ApprovedItemInstall {
                    pack_id: &pack_id,
                    kind,
                    id: &id,
                    document,
                    expected: expected_revision,
                    attestation_id: &attestation_id,
                    source: &source,
                },
            ),
            PackRequest::ItemDelete {
                pack_id,
                kind,
                id,
                expected_revision,
            } => delete_item(&mut store, &pack_id, kind, &id, expected_revision),
            PackRequest::ItemOrigin {
                pack_id,
                kind,
                id,
                revision,
            } => item_origin(&store, &pack_id, kind, &id, revision),
        }
    }

    pub fn context(&self, request: ContextRequest) -> Result<Value> {
        let store = Store::open(&self.store_path)?;
        let (pack_id, pack_revision, context) =
            store.serve_context(&request.name, request.pack.as_deref())?;
        let origin =
            store.item_origin(&pack_id, pack_revision, ItemKind::Context, &context.name)?;
        Ok(
            json!({"pack_id":pack_id,"pack_revision":pack_revision,"context":context,"origin":origin}),
        )
    }

    pub fn verb(&self, request: VerbRequest) -> Result<Value> {
        let store = Store::open(&self.store_path)?;
        match request {
            VerbRequest::Get { name, pack } => {
                let (pack_id, pack_revision, verb) = store.serve_verb(&name, pack.as_deref())?;
                let origin =
                    store.item_origin(&pack_id, pack_revision, ItemKind::Verb, &verb.name)?;
                Ok(
                    json!({"pack_id":pack_id,"pack_revision":pack_revision,"verb":verb,"origin":origin}),
                )
            }
            VerbRequest::List { context, pack } => {
                let items = store.list_verbs(context.as_deref(), pack.as_deref())?.into_iter()
                    .map(|(pack_id, pack_revision, verb)| {
                        let origin = store.item_origin(&pack_id, pack_revision, ItemKind::Verb, &verb.name)?;
                        Ok(json!({"pack_id":pack_id,"pack_revision":pack_revision,"verb":verb,"origin":origin}))
                    }).collect::<Result<Vec<_>>>()?;
                Ok(json!({"items":items}))
            }
        }
    }

    pub fn surface(&self, request: SurfaceRequest) -> Result<Value> {
        let store = Store::open(&self.store_path)?;
        let resolved = store.serve_surface(&request.name, request.pack.as_deref())?;
        let surface_origin = store.item_origin(
            &resolved.pack_id,
            resolved.pack_revision,
            ItemKind::Surface,
            &resolved.surface,
        )?;
        let context_origin = store.item_origin(
            &resolved.pack_id,
            resolved.pack_revision,
            ItemKind::Context,
            &resolved.context.name,
        )?;
        let default_audience_tier_origin = store.item_origin(
            &resolved.pack_id,
            resolved.pack_revision,
            ItemKind::Tier,
            &resolved.default_audience_tier.name,
        )?;
        let mut value = serde_json::to_value(resolved)?;
        let object = value
            .as_object_mut()
            .expect("surface resolution serializes as an object");
        object.insert("surface_origin".into(), to_value(surface_origin)?);
        object.insert("context_origin".into(), to_value(context_origin)?);
        object.insert(
            "default_audience_tier_origin".into(),
            to_value(default_audience_tier_origin)?,
        );
        Ok(value)
    }

    pub fn draft(&self, request: DraftRequest) -> Result<Value> {
        let mut store = Store::open(&self.store_path)?;
        match request {
            DraftRequest::Create { draft } => mutation("created", store.create_draft(&draft)?),
            DraftRequest::Append {
                id,
                parent,
                content,
                author_kind,
                provenance,
            } => mutation(
                "version-appended",
                store.revise_draft_with(&id, parent, &content, author_kind, &provenance)?,
            ),
            DraftRequest::Get { id, version } => to_value(store.get_draft(&id, version)?),
            DraftRequest::Versions { id } => Ok(json!({"items":store.list_draft_versions(&id)?})),
        }
    }

    /// Atomically freezes one assist material revision and records the source
    /// selection before any transport invokes an external backend.
    pub fn prepare_assist(&self, request: &AssistPreparation) -> Result<PreparedAssist> {
        Store::open(&self.store_path)?.prepare_assist(request)
    }

    /// Records an attached agent's result against the exact prepared source.
    pub fn complete_assist(
        &self,
        source: &DraftTarget,
        candidate: &str,
        provenance: &Value,
    ) -> Result<DraftVersion> {
        Store::open(&self.store_path)?.revise_draft_with(
            &source.draft_id,
            source.version,
            candidate,
            AuthorKind::Agent,
            provenance,
        )
    }

    pub fn attestation(&self, request: AttestationRequest) -> Result<Value> {
        let mut store = Store::open(&self.store_path)?;
        match request {
            AttestationRequest::Record { attestation } => {
                mutation("recorded", store.attest(&attestation)?)
            }
            AttestationRequest::Show { id } => {
                let items = store.list_attestations(None, Some(&id))?;
                if items.is_empty() {
                    return Err(Error::NotFound {
                        kind: "attestation",
                        id,
                    });
                }
                Ok(json!({"id":id,"items":items}))
            }
            AttestationRequest::List { draft, version } => {
                let mut items = store.list_attestations(draft.as_deref(), None)?;
                if let Some(version) = version {
                    items.retain(|item| item.draft.version == version);
                }
                Ok(json!({"items":items}))
            }
        }
    }

    pub fn capture(&self, request: CaptureRequest) -> Result<Value> {
        let mut store = Store::open(&self.store_path)?;
        match request {
            CaptureRequest::Record { capture } => {
                mutation("recorded", store.record_capture(&capture)?)
            }
            CaptureRequest::Get { id } => to_value(store.get_capture(&id)?),
            CaptureRequest::List { surface } => {
                Ok(json!({"items":store.list_captures(surface.as_deref())?}))
            }
        }
    }

    pub fn render(&self, request: RenderRequest) -> Result<Value> {
        match request.host {
            Host::Opencode => Ok(json!({
                "host":"opencode",
                "content":OPENCODE_POINTER,
            })),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PackRequest {
    Create {
        document: PackDocument,
    },
    Update {
        document: PackDocument,
        expected_revision: u64,
    },
    Get {
        id: String,
        revision: Option<u64>,
    },
    List,
    Delete {
        id: String,
        expected_revision: u64,
    },
    Use {
        id: String,
    },
    Import {
        document: PackDocument,
        expected_revision: Option<u64>,
        #[serde(default)]
        activate: bool,
    },
    Export {
        id: String,
        revision: Option<u64>,
    },
    ItemPut {
        pack_id: String,
        kind: ItemKind,
        id: String,
        document: Value,
        expected_revision: u64,
    },
    ItemImport {
        pack_id: String,
        kind: ItemKind,
        id: String,
        document: Value,
        expected_revision: u64,
        source_label: Option<String>,
        source_uri: Option<String>,
        #[serde(default = "default_import_channel")]
        channel: String,
    },
    ItemInstallApprovedDraft {
        pack_id: String,
        kind: ItemKind,
        id: String,
        document: Value,
        expected_revision: u64,
        attestation_id: String,
        source: DraftTarget,
    },
    ItemDelete {
        pack_id: String,
        kind: ItemKind,
        id: String,
        expected_revision: u64,
    },
    ItemOrigin {
        pack_id: String,
        kind: ItemKind,
        id: String,
        revision: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextRequest {
    pub name: String,
    pub pack: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum VerbRequest {
    Get {
        name: String,
        pack: Option<String>,
    },
    List {
        context: Option<String>,
        pack: Option<String>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceRequest {
    pub name: String,
    pub pack: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum DraftRequest {
    Create {
        draft: DraftCreate,
    },
    Append {
        id: String,
        parent: u64,
        content: String,
        author_kind: AuthorKind,
        #[serde(default = "empty_object")]
        provenance: Value,
    },
    Get {
        id: String,
        version: Option<u64>,
    },
    Versions {
        id: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum AttestationRequest {
    Record {
        attestation: AttestationInput,
    },
    Show {
        id: String,
    },
    List {
        draft: Option<String>,
        version: Option<u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "action", rename_all = "kebab-case", deny_unknown_fields)]
pub enum CaptureRequest {
    Record { capture: CaptureInput },
    Get { id: String },
    List { surface: Option<String> },
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    pub host: Host,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum Host {
    Opencode,
}

pub const OPENCODE_POINTER: &str = r#"# Route human-facing writing through prose

Before posting content to a human audience, fetch the current context module and verb specification from prose rather than relying on copied guidance.
Use the prose draft tools to record agent and human versions.
Ask the human to approve the exact final version.
Record an attestation only after the human explicitly approves it.
Posting tools remain independent; the agent composes this workflow.
Never treat an agent response as human approval.
"#;

struct ExternalItemRevision<'a> {
    pack_id: &'a str,
    kind: ItemKind,
    id: &'a str,
    document: Option<Value>,
    expected: u64,
    source_label: Option<&'a str>,
    source_uri: Option<&'a str>,
    channel: &'a str,
}

fn revise_external_item(store: &mut Store, revision: ExternalItemRevision<'_>) -> Result<Value> {
    let mut pack = store.get_pack(revision.pack_id, None)?.document;
    let caller_material = revision.document.clone().unwrap_or(Value::Null);
    match (revision.kind, revision.document) {
        (ItemKind::Context, Some(value)) => replace(
            &mut pack.contexts,
            parse_item::<ContextSpec>(value, revision.id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Verb, Some(value)) => replace(
            &mut pack.verbs,
            parse_item::<VerbSpec>(value, revision.id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Surface, Some(value)) => replace(
            &mut pack.surface_mappings,
            parse_item::<SurfaceMapping>(value, revision.id, |v| &v.surface)?,
            |v| &v.surface,
        ),
        (ItemKind::Tier, Some(value)) => replace(
            &mut pack.audience_tiers,
            parse_item::<AudienceTier>(value, revision.id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Context, None) => remove(&mut pack.contexts, revision.id, |v| &v.name)?,
        (ItemKind::Verb, None) => remove(&mut pack.verbs, revision.id, |v| &v.name)?,
        (ItemKind::Surface, None) => {
            remove(&mut pack.surface_mappings, revision.id, |v| &v.surface)?
        }
        (ItemKind::Tier, None) => remove(&mut pack.audience_tiers, revision.id, |v| &v.name)?,
    }
    mutation(
        "updated",
        store.update_pack_item_external(
            &pack,
            revision.expected,
            revision.kind,
            revision.id,
            &caller_material,
            revision.source_label,
            revision.source_uri,
            revision.channel,
        )?,
    )
}

struct ApprovedItemInstall<'a> {
    pack_id: &'a str,
    kind: ItemKind,
    id: &'a str,
    document: Value,
    expected: u64,
    attestation_id: &'a str,
    source: &'a DraftTarget,
}

fn install_approved_item(store: &mut Store, install: ApprovedItemInstall<'_>) -> Result<Value> {
    let mut pack = store.get_pack(install.pack_id, None)?.document;
    match install.kind {
        ItemKind::Context => {
            let item = parse_item::<ContextSpec>(install.document, install.id, |v| &v.name)?;
            replace(&mut pack.contexts, item, |v| &v.name);
        }
        ItemKind::Verb => {
            let item = parse_item::<VerbSpec>(install.document, install.id, |v| &v.name)?;
            replace(&mut pack.verbs, item, |v| &v.name);
        }
        ItemKind::Tier | ItemKind::Surface => {
            return Err(Error::Validation(
                "approved-draft installs support only context and verb items".into(),
            ));
        }
    }
    mutation(
        "installed",
        store.update_pack_item_approved(
            &pack,
            install.expected,
            install.kind,
            install.id,
            install.attestation_id,
            install.source,
        )?,
    )
}

fn item_origin(
    store: &Store,
    pack_id: &str,
    kind: ItemKind,
    id: &str,
    revision: Option<u64>,
) -> Result<Value> {
    let pack = store.get_pack(pack_id, revision)?;
    let exists = match kind {
        ItemKind::Context => pack.document.contexts.iter().any(|item| item.name == id),
        ItemKind::Verb => pack.document.verbs.iter().any(|item| item.name == id),
        ItemKind::Tier => pack
            .document
            .audience_tiers
            .iter()
            .any(|item| item.name == id),
        ItemKind::Surface => pack
            .document
            .surface_mappings
            .iter()
            .any(|item| item.surface == id),
    };
    if !exists {
        return Err(Error::NotFound {
            kind: "pack item",
            id: id.into(),
        });
    }
    let origin = store.item_origin(pack_id, pack.revision, kind, id)?;
    Ok(json!({
        "pack_id": pack_id,
        "pack_revision": pack.revision,
        "kind": kind,
        "id": id,
        "origin": origin,
    }))
}

fn delete_item(
    store: &mut Store,
    pack_id: &str,
    kind: ItemKind,
    id: &str,
    expected: u64,
) -> Result<Value> {
    let mut pack = store.get_pack(pack_id, None)?.document;
    match kind {
        ItemKind::Context => remove(&mut pack.contexts, id, |v| &v.name)?,
        ItemKind::Verb => remove(&mut pack.verbs, id, |v| &v.name)?,
        ItemKind::Surface => remove(&mut pack.surface_mappings, id, |v| &v.surface)?,
        ItemKind::Tier => remove(&mut pack.audience_tiers, id, |v| &v.name)?,
    }
    mutation(
        "updated",
        store.update_pack_item_deleted(&pack, expected, kind, id)?,
    )
}

fn parse<T: DeserializeOwned>(value: Value) -> Result<T> {
    Ok(serde_json::from_value(value)?)
}
fn parse_item<T: DeserializeOwned>(
    value: Value,
    id: &str,
    key: impl Fn(&T) -> &String,
) -> Result<T> {
    let item: T = parse(value)?;
    if key(&item) != id {
        return Err(Error::Validation(format!("item id does not match `{id}`")));
    }
    Ok(item)
}
fn mutation(action: &str, item: impl Serialize) -> Result<Value> {
    Ok(json!({"action":action,"item":item}))
}
fn to_value(value: impl Serialize) -> Result<Value> {
    Ok(serde_json::to_value(value)?)
}
fn replace<T>(items: &mut Vec<T>, item: T, key: impl Fn(&T) -> &String) {
    if let Some(index) = items.iter().position(|old| key(old) == key(&item)) {
        items[index] = item;
    } else {
        items.push(item);
    }
}
fn remove<T>(items: &mut Vec<T>, id: &str, key: impl Fn(&T) -> &String) -> Result<()> {
    let before = items.len();
    items.retain(|item| key(item) != id);
    if before == items.len() {
        Err(Error::NotFound {
            kind: "pack item",
            id: id.into(),
        })
    } else {
        Ok(())
    }
}
fn empty_object() -> Value {
    json!({})
}
fn default_import_channel() -> String {
    "application".into()
}
