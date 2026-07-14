use crate::{
    AttestationInput, AudienceTier, AuthorKind, ContextSpec, DraftCreate, Error, PackDocument,
    Result, Store, SurfaceMapping, VerbSpec,
};
use clap::{Args, Parser, Subcommand, ValueEnum};
use serde::{Serialize, de::DeserializeOwned};
use serde_json::{Value, json};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
};

#[derive(Debug, Parser)]
#[command(
    name = "prose",
    version,
    about = "Deterministic voice-pack, draft, and approval broker"
)]
pub struct Cli {
    /// Print exactly one compact JSON document.
    #[arg(long, global = true)]
    pub json: bool,
    /// SQLite store path (otherwise XDG_DATA_HOME/prose/prose.db).
    #[arg(long, global = true, env = "PROSE_STORE")]
    pub store: Option<PathBuf>,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    Pack {
        #[command(subcommand)]
        command: PackCommand,
    },
    Context(NamedServe),
    Verb(NamedServe),
    /// Enumerate verb specifications in the selected pack.
    Verbs(VerbList),
    Surface(NamedServe),
    Draft {
        #[command(subcommand)]
        command: DraftCommand,
    },
    Attestation {
        #[command(subcommand)]
        command: AttestationCommand,
    },
    /// Record an immutable human-approval attestation.
    Attest(InputFile),
}

#[derive(Debug, Args)]
pub struct NamedServe {
    pub name: String,
    /// Pack ID; otherwise use the explicitly selected active pack.
    #[arg(long)]
    pub pack: Option<String>,
}

#[derive(Debug, Args)]
pub struct VerbList {
    #[arg(long)]
    pub context: Option<String>,
    #[arg(long)]
    pub pack: Option<String>,
}

#[derive(Debug, Subcommand)]
pub enum PackCommand {
    Create(InputFile),
    Update {
        #[arg(long, alias = "if-revision")]
        expected_revision: u64,
        #[command(flatten)]
        input: InputFile,
    },
    #[command(alias = "show")]
    Get {
        id: String,
        #[arg(long)]
        revision: Option<u64>,
    },
    List,
    Delete {
        id: String,
        #[arg(long, alias = "if-revision")]
        expected_revision: u64,
    },
    /// Select the pack used by unqualified serve commands.
    Use {
        id: String,
    },
    /// Import a complete frozen pack document.
    Import {
        #[command(flatten)]
        input: InputFile,
        #[arg(long, alias = "if-revision")]
        expected_revision: Option<u64>,
        #[arg(long)]
        activate: bool,
    },
    /// Export a complete frozen pack document.
    Export {
        id: String,
        #[arg(long)]
        revision: Option<u64>,
    },
    /// Revise one typed item while preserving a complete pack snapshot.
    Item {
        #[command(subcommand)]
        command: PackItemCommand,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ItemKind {
    Context,
    Verb,
    Surface,
    Tier,
}

#[derive(Debug, Subcommand)]
pub enum PackItemCommand {
    Put {
        pack_id: String,
        #[arg(long)]
        kind: ItemKind,
        #[arg(long)]
        id: String,
        #[arg(long, default_value = "-")]
        file: PathBuf,
        #[arg(long, alias = "expected-revision")]
        if_revision: u64,
    },
    Delete {
        pack_id: String,
        #[arg(long)]
        kind: ItemKind,
        #[arg(long)]
        id: String,
        #[arg(long, alias = "expected-revision")]
        if_revision: u64,
    },
}

#[derive(Debug, Clone, Args)]
pub struct InputFile {
    /// JSON input file, or '-' for stdin.
    #[arg(long, default_value = "-")]
    pub file: PathBuf,
}

#[derive(Debug, Subcommand)]
pub enum DraftCommand {
    Create(InputFile),
    #[command(alias = "revise")]
    Append {
        id: String,
        #[arg(long, alias = "expected-version")]
        parent: u64,
        #[arg(long, default_value = "-", alias = "content")]
        content_file: PathBuf,
        #[arg(long, value_enum, default_value = "human")]
        author_kind: AuthorKind,
        #[arg(long)]
        provenance_file: Option<PathBuf>,
    },
    #[command(alias = "show")]
    Get {
        id: String,
        #[arg(long, alias = "version")]
        version: Option<u64>,
    },
    Versions {
        id: String,
    },
}

#[derive(Debug, Subcommand)]
pub enum AttestationCommand {
    Record(InputFile),
    Show {
        id: String,
    },
    List {
        #[arg(long)]
        draft: Option<String>,
        #[arg(long)]
        version: Option<u64>,
    },
}

pub struct Output {
    pub json: bool,
    pub value: Value,
}

pub fn execute(cli: Cli) -> Result<Output> {
    let json_output = cli.json;
    let path = cli.store.map(Ok).unwrap_or_else(Store::default_path)?;
    let mut store = Store::open(path)?;
    let value = match cli.command {
        Command::Pack { command } => execute_pack(&mut store, command)?,
        Command::Context(args) => {
            let (pack_id, pack_revision, context) =
                store.serve_context(&args.name, args.pack.as_deref())?;
            json!({"pack_id":pack_id,"pack_revision":pack_revision,"context":context})
        }
        Command::Verb(args) => {
            let (pack_id, pack_revision, verb) =
                store.serve_verb(&args.name, args.pack.as_deref())?;
            json!({"pack_id":pack_id,"pack_revision":pack_revision,"verb":verb})
        }
        Command::Verbs(args) => {
            let items = store.list_verbs(args.context.as_deref(), args.pack.as_deref())?.into_iter()
                .map(|(pack_id, pack_revision, verb)| json!({"pack_id":pack_id,"pack_revision":pack_revision,"verb":verb})).collect::<Vec<_>>();
            json!({"items":items})
        }
        Command::Surface(args) => to_value(store.serve_surface(&args.name, args.pack.as_deref())?)?,
        Command::Draft { command } => execute_draft(&mut store, command)?,
        Command::Attestation { command } => execute_attestation(&mut store, command)?,
        Command::Attest(input) => mutation(
            "recorded",
            store.attest(&read_json::<AttestationInput>(&input.file)?)?,
        )?,
    };
    Ok(Output {
        json: json_output,
        value,
    })
}

fn execute_pack(store: &mut Store, command: PackCommand) -> Result<Value> {
    match command {
        PackCommand::Create(input) => {
            mutation("created", store.create_pack(&read_json(&input.file)?)?)
        }
        PackCommand::Update {
            expected_revision,
            input,
        } => mutation(
            "updated",
            store.update_pack(&read_json(&input.file)?, expected_revision)?,
        ),
        PackCommand::Get { id, revision } => to_value(store.get_pack(&id, revision)?),
        PackCommand::List => Ok(json!({"items":store.list_packs()?})),
        PackCommand::Delete {
            id,
            expected_revision,
        } => mutation("deleted", store.delete_pack(&id, expected_revision)?),
        PackCommand::Use { id } => mutation("selected", store.use_pack(&id)?),
        PackCommand::Import {
            input,
            expected_revision,
            activate,
        } => {
            let document: PackDocument = read_json(&input.file)?;
            let pack = match expected_revision {
                Some(revision) => store.update_pack(&document, revision)?,
                None => store.create_pack(&document)?,
            };
            if activate {
                store.use_pack(&document.id)?;
            }
            mutation("imported", pack)
        }
        PackCommand::Export { id, revision } => to_value(store.get_pack(&id, revision)?.document),
        PackCommand::Item { command } => execute_pack_item(store, command),
    }
}

fn execute_pack_item(store: &mut Store, command: PackItemCommand) -> Result<Value> {
    let (pack_id, kind, id, expected, document) = match command {
        PackItemCommand::Put {
            pack_id,
            kind,
            id,
            file,
            if_revision,
        } => (pack_id, kind, id, if_revision, Some(read_text(&file)?)),
        PackItemCommand::Delete {
            pack_id,
            kind,
            id,
            if_revision,
        } => (pack_id, kind, id, if_revision, None),
    };
    let mut pack = store.get_pack(&pack_id, None)?.document;
    match (kind, document) {
        (ItemKind::Context, Some(json)) => replace(
            &mut pack.contexts,
            parse_item::<ContextSpec>(&json, &id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Verb, Some(json)) => replace(
            &mut pack.verbs,
            parse_item::<VerbSpec>(&json, &id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Surface, Some(json)) => replace(
            &mut pack.surface_mappings,
            parse_item::<SurfaceMapping>(&json, &id, |v| &v.surface)?,
            |v| &v.surface,
        ),
        (ItemKind::Tier, Some(json)) => replace(
            &mut pack.audience_tiers,
            parse_item::<AudienceTier>(&json, &id, |v| &v.name)?,
            |v| &v.name,
        ),
        (ItemKind::Context, None) => remove(&mut pack.contexts, &id, |v| &v.name)?,
        (ItemKind::Verb, None) => remove(&mut pack.verbs, &id, |v| &v.name)?,
        (ItemKind::Surface, None) => remove(&mut pack.surface_mappings, &id, |v| &v.surface)?,
        (ItemKind::Tier, None) => remove(&mut pack.audience_tiers, &id, |v| &v.name)?,
    }
    mutation("updated", store.update_pack(&pack, expected)?)
}

fn execute_draft(store: &mut Store, command: DraftCommand) -> Result<Value> {
    match command {
        DraftCommand::Create(input) => mutation(
            "created",
            store.create_draft(&read_json::<DraftCreate>(&input.file)?)?,
        ),
        DraftCommand::Append {
            id,
            parent,
            content_file,
            author_kind,
            provenance_file,
        } => {
            let provenance = provenance_file
                .map(|path| read_json::<Value>(&path))
                .transpose()?
                .unwrap_or_else(|| json!({}));
            mutation(
                "version-appended",
                store.revise_draft_with(
                    &id,
                    parent,
                    &read_text(&content_file)?,
                    author_kind,
                    &provenance,
                )?,
            )
        }
        DraftCommand::Get { id, version } => to_value(store.get_draft(&id, version)?),
        DraftCommand::Versions { id } => Ok(json!({"items":store.list_draft_versions(&id)?})),
    }
}

fn execute_attestation(store: &mut Store, command: AttestationCommand) -> Result<Value> {
    match command {
        AttestationCommand::Record(input) => mutation(
            "recorded",
            store.attest(&read_json::<AttestationInput>(&input.file)?)?,
        ),
        AttestationCommand::Show { id } => {
            let items = store.list_attestations(None, Some(&id))?;
            if items.is_empty() {
                return Err(Error::NotFound {
                    kind: "attestation",
                    id,
                });
            }
            Ok(json!({"id":id,"items":items}))
        }
        AttestationCommand::List { draft, version } => {
            let mut items = store.list_attestations(draft.as_deref(), None)?;
            if let Some(version) = version {
                items.retain(|item| item.draft.version == version);
            }
            Ok(json!({"items":items}))
        }
    }
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
fn parse_item<T: DeserializeOwned>(text: &str, id: &str, key: impl Fn(&T) -> &String) -> Result<T> {
    let item: T = serde_json::from_str(text)?;
    if key(&item) != id {
        return Err(Error::Validation(format!("item id does not match `{id}`")));
    }
    Ok(item)
}
fn read_text(path: &Path) -> Result<String> {
    if path == Path::new("-") {
        let mut value = String::new();
        std::io::stdin().read_to_string(&mut value)?;
        Ok(value)
    } else {
        Ok(fs::read_to_string(path)?)
    }
}
fn read_json<T: DeserializeOwned>(path: &Path) -> Result<T> {
    let text = read_text(path)?;
    let mut de = serde_json::Deserializer::from_str(&text);
    let value = T::deserialize(&mut de)?;
    de.end()?;
    Ok(value)
}
