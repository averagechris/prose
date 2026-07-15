//! Model Context Protocol transport for the transport-neutral application API.

use crate::{
    Application, AttestationRequest, ContextRequest, DraftRequest, PackRequest, RenderRequest,
    SurfaceRequest, VerbRequest,
};
use rmcp::{
    ErrorData as McpError, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use std::path::PathBuf;

#[derive(Clone)]
pub struct ProseMcp {
    app: Application,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct PackParams {
    request: PackRequest,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct VerbParams {
    request: VerbRequest,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct DraftParams {
    request: DraftRequest,
}

#[derive(Debug, Deserialize, JsonSchema)]
struct AttestationParams {
    request: AttestationRequest,
}

#[tool_router]
impl ProseMcp {
    pub fn new(app: Application) -> Self {
        Self { app }
    }

    #[tool(
        description = "Create, update, inspect, select, import, export, or tombstone voice packs and their typed items"
    )]
    fn pack(&self, Parameters(params): Parameters<PackParams>) -> Result<String, McpError> {
        tool_result(self.app.pack(params.request))
    }

    #[tool(description = "Serve an explicit context module from a named or active pack")]
    fn context(&self, Parameters(request): Parameters<ContextRequest>) -> Result<String, McpError> {
        tool_result(self.app.context(request))
    }

    #[tool(
        description = "Serve or list verb specifications from a named or active pack; prose does not execute the instructions"
    )]
    fn verb(&self, Parameters(params): Parameters<VerbParams>) -> Result<String, McpError> {
        tool_result(self.app.verb(params.request))
    }

    #[tool(
        description = "Resolve a surface to its context module and audience tier deterministically"
    )]
    fn surface(&self, Parameters(request): Parameters<SurfaceRequest>) -> Result<String, McpError> {
        tool_result(self.app.surface(request))
    }

    #[tool(description = "Create, append, or inspect linear draft version chains with provenance")]
    fn draft(&self, Parameters(params): Parameters<DraftParams>) -> Result<String, McpError> {
        tool_result(self.app.draft(params.request))
    }

    #[tool(
        description = "Record reported human approval or inspect immutable approval attestations"
    )]
    fn attestation(
        &self,
        Parameters(params): Parameters<AttestationParams>,
    ) -> Result<String, McpError> {
        tool_result(self.app.attestation(params.request))
    }

    #[tool(description = "Render a small host adapter that points back to live prose tools")]
    fn render(&self, Parameters(request): Parameters<RenderRequest>) -> Result<String, McpError> {
        tool_result(self.app.render(request))
    }
}

#[tool_handler]
impl ServerHandler for ProseMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("prose", env!("CARGO_PKG_VERSION")))
            .with_instructions("Deterministic, model-free access to prose packs, drafts, verbs, and human approval attestations. All judgment remains in the attached agent runtime.")
    }
}

pub async fn run(store_path: PathBuf) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let service = ProseMcp::new(Application::new(store_path))
        .serve(stdio())
        .await?;
    service.waiting().await?;
    Ok(())
}

fn mcp_error(error: crate::Error) -> McpError {
    let data = Some(json!({"code":error.code()}));
    match error {
        crate::Error::Sql(_) | crate::Error::Io(_) => {
            McpError::internal_error(error.to_string(), data)
        }
        _ => McpError::invalid_params(error.to_string(), data),
    }
}

fn tool_result(result: crate::Result<Value>) -> Result<String, McpError> {
    result.map(|value| value.to_string()).map_err(mcp_error)
}
