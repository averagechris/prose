use crate::{
    Application, AttestationInput, AttestationRequest, AuthorKind, ContextRequest, DraftCreate,
    DraftRequest, Host, ItemKind, PackDocument, PackRequest, RenderRequest, Result, Store,
    SurfaceRequest, VerbRequest,
};
use clap::{Args, Parser, Subcommand};
use serde::de::DeserializeOwned;
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
    /// Run the Model Context Protocol server over stdio.
    Mcp,
    /// Render a small host adapter that points back to live prose tools.
    Render {
        #[arg(long, value_enum)]
        host: Host,
    },
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
    pub plain: Option<String>,
}

pub fn execute(cli: Cli) -> Result<Output> {
    let json_output = cli.json;
    let path = cli.store.map(Ok).unwrap_or_else(Store::default_path)?;
    let app = Application::new(path);
    let mut plain = None;
    let value = match cli.command {
        Command::Pack { command } => execute_pack(&app, command)?,
        Command::Context(args) => app.context(ContextRequest {
            name: args.name,
            pack: args.pack,
        })?,
        Command::Verb(args) => app.verb(VerbRequest::Get {
            name: args.name,
            pack: args.pack,
        })?,
        Command::Verbs(args) => app.verb(VerbRequest::List {
            context: args.context,
            pack: args.pack,
        })?,
        Command::Surface(args) => app.surface(SurfaceRequest {
            name: args.name,
            pack: args.pack,
        })?,
        Command::Draft { command } => execute_draft(&app, command)?,
        Command::Attestation { command } => execute_attestation(&app, command)?,
        Command::Attest(input) => app.attestation(AttestationRequest::Record {
            attestation: read_json::<AttestationInput>(&input.file)?,
        })?,
        Command::Render { host } => {
            let rendered = app.render(RenderRequest { host })?;
            plain = rendered
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_owned);
            rendered
        }
        Command::Mcp => {
            return Err(crate::Error::Conflict(
                "MCP must be run as a transport".into(),
            ));
        }
    };
    Ok(Output {
        json: json_output,
        value,
        plain,
    })
}

fn execute_pack(app: &Application, command: PackCommand) -> Result<Value> {
    match command {
        PackCommand::Create(input) => app.pack(PackRequest::Create {
            document: read_json::<PackDocument>(&input.file)?,
        }),
        PackCommand::Update {
            expected_revision,
            input,
        } => app.pack(PackRequest::Update {
            document: read_json::<PackDocument>(&input.file)?,
            expected_revision,
        }),
        PackCommand::Get { id, revision } => app.pack(PackRequest::Get { id, revision }),
        PackCommand::List => app.pack(PackRequest::List),
        PackCommand::Delete {
            id,
            expected_revision,
        } => app.pack(PackRequest::Delete {
            id,
            expected_revision,
        }),
        PackCommand::Use { id } => app.pack(PackRequest::Use { id }),
        PackCommand::Import {
            input,
            expected_revision,
            activate,
        } => app.pack(PackRequest::Import {
            document: read_json::<PackDocument>(&input.file)?,
            expected_revision,
            activate,
        }),
        PackCommand::Export { id, revision } => app.pack(PackRequest::Export { id, revision }),
        PackCommand::Item { command } => execute_pack_item(app, command),
    }
}

fn execute_pack_item(app: &Application, command: PackItemCommand) -> Result<Value> {
    match command {
        PackItemCommand::Put {
            pack_id,
            kind,
            id,
            file,
            if_revision,
        } => app.pack(PackRequest::ItemPut {
            pack_id,
            kind,
            id,
            document: read_value(&file)?,
            expected_revision: if_revision,
        }),
        PackItemCommand::Delete {
            pack_id,
            kind,
            id,
            if_revision,
        } => app.pack(PackRequest::ItemDelete {
            pack_id,
            kind,
            id,
            expected_revision: if_revision,
        }),
    }
}

fn execute_draft(app: &Application, command: DraftCommand) -> Result<Value> {
    match command {
        DraftCommand::Create(input) => app.draft(DraftRequest::Create {
            draft: read_json::<DraftCreate>(&input.file)?,
        }),
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
            app.draft(DraftRequest::Append {
                id,
                parent,
                content: read_text(&content_file)?,
                author_kind,
                provenance,
            })
        }
        DraftCommand::Get { id, version } => app.draft(DraftRequest::Get { id, version }),
        DraftCommand::Versions { id } => app.draft(DraftRequest::Versions { id }),
    }
}

fn execute_attestation(app: &Application, command: AttestationCommand) -> Result<Value> {
    match command {
        AttestationCommand::Record(input) => app.attestation(AttestationRequest::Record {
            attestation: read_json::<AttestationInput>(&input.file)?,
        }),
        AttestationCommand::Show { id } => app.attestation(AttestationRequest::Show { id }),
        AttestationCommand::List { draft, version } => {
            app.attestation(AttestationRequest::List { draft, version })
        }
    }
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
fn read_value(path: &Path) -> Result<Value> {
    read_json(path)
}
