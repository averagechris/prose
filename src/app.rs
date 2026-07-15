//! Transport-neutral prose operations shared by the CLI and MCP server.

use crate::{
    AttestationInput, AudienceTier, AuthorKind, ContextSpec, DraftCreate, Error, PackDocument,
    Result, Store, SurfaceMapping, VerbSpec,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

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
                    Some(revision) => store.update_pack(&document, revision)?,
                    None => store.create_pack(&document)?,
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
            } => revise_item(
                &mut store,
                &pack_id,
                kind,
                &id,
                Some(document),
                expected_revision,
            ),
            PackRequest::ItemDelete {
                pack_id,
                kind,
                id,
                expected_revision,
            } => revise_item(&mut store, &pack_id, kind, &id, None, expected_revision),
        }
    }

    pub fn context(&self, request: ContextRequest) -> Result<Value> {
        let store = Store::open(&self.store_path)?;
        let (pack_id, pack_revision, context) =
            store.serve_context(&request.name, request.pack.as_deref())?;
        Ok(json!({"pack_id":pack_id,"pack_revision":pack_revision,"context":context}))
    }

    pub fn verb(&self, request: VerbRequest) -> Result<Value> {
        let store = Store::open(&self.store_path)?;
        match request {
            VerbRequest::Get { name, pack } => {
                let (pack_id, pack_revision, verb) = store.serve_verb(&name, pack.as_deref())?;
                Ok(json!({"pack_id":pack_id,"pack_revision":pack_revision,"verb":verb}))
            }
            VerbRequest::List { context, pack } => {
                let items = store.list_verbs(context.as_deref(), pack.as_deref())?.into_iter()
                    .map(|(pack_id, pack_revision, verb)| json!({"pack_id":pack_id,"pack_revision":pack_revision,"verb":verb})).collect::<Vec<_>>();
                Ok(json!({"items":items}))
            }
        }
    }

    pub fn surface(&self, request: SurfaceRequest) -> Result<Value> {
        let store = Store::open(&self.store_path)?;
        to_value(store.serve_surface(&request.name, request.pack.as_deref())?)
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
    ItemDelete {
        pack_id: String,
        kind: ItemKind,
        id: String,
        expected_revision: u64,
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
        #[serde(default)]
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
#[serde(deny_unknown_fields)]
pub struct RenderRequest {
    pub host: Host,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum ItemKind {
    Context,
    Verb,
    Surface,
    Tier,
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

fn revise_item(
    store: &mut Store,
    pack_id: &str,
    kind: ItemKind,
    id: &str,
    document: Option<Value>,
    expected: u64,
) -> Result<Value> {
    let mut pack = store.get_pack(pack_id, None)?.document;
    match (kind, document) {
        (ItemKind::Context, Some(value)) => replace(
            &mut pack.contexts,
            parse_item::<ContextSpec>(value, id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Verb, Some(value)) => replace(
            &mut pack.verbs,
            parse_item::<VerbSpec>(value, id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Surface, Some(value)) => replace(
            &mut pack.surface_mappings,
            parse_item::<SurfaceMapping>(value, id, |v| &v.surface)?,
            |v| &v.surface,
        ),
        (ItemKind::Tier, Some(value)) => replace(
            &mut pack.audience_tiers,
            parse_item::<AudienceTier>(value, id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Context, None) => remove(&mut pack.contexts, id, |v| &v.name)?,
        (ItemKind::Verb, None) => remove(&mut pack.verbs, id, |v| &v.name)?,
        (ItemKind::Surface, None) => remove(&mut pack.surface_mappings, id, |v| &v.surface)?,
        (ItemKind::Tier, None) => remove(&mut pack.audience_tiers, id, |v| &v.name)?,
    }
    mutation("updated", store.update_pack(&pack, expected)?)
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
