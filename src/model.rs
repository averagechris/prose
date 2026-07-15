use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContextSpec {
    pub name: String,
    pub description: String,
    pub content: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum VerbFamily {
    TransformSelection,
    DraftFromIntent,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum LengthUnit {
    Characters,
    Words,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LengthConstraint {
    pub unit: LengthUnit,
    pub min: Option<u64>,
    pub max: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerbConstraints {
    pub length: Option<LengthConstraint>,
    pub no_new_claims: bool,
    pub preserve_meaning: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct VerbSpec {
    pub name: String,
    pub description: String,
    pub family: VerbFamily,
    pub instructions: String,
    #[serde(default)]
    pub context_bindings: Vec<String>,
    pub constraints: VerbConstraints,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AudienceTier {
    pub name: String,
    pub description: String,
    #[serde(default)]
    pub requirements: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfaceMapping {
    pub surface: String,
    pub context: String,
    pub default_audience_tier: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SurfaceResolution {
    pub pack_id: String,
    pub pack_revision: u64,
    pub surface: String,
    pub context: ContextSpec,
    pub default_audience_tier: AudienceTier,
}

/// Provenance recorded for a schema-v3 pack-item revision.
///
/// Older pack-item revisions can legitimately have no origin.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "kebab-case", deny_unknown_fields)]
pub enum PackItemOrigin {
    ApprovedDraft {
        attestation_id: String,
        draft_id: String,
        draft_version: u64,
    },
    ExternalImport {
        source_label: Option<String>,
        source_uri: Option<String>,
        material_sha256: String,
        channel: String,
        recorded_at: String,
    },
}

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, clap::ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum ItemKind {
    Context,
    Verb,
    Surface,
    Tier,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PackDocument {
    pub id: String,
    pub description: String,
    #[serde(default)]
    pub contexts: Vec<ContextSpec>,
    #[serde(default)]
    pub verbs: Vec<VerbSpec>,
    #[serde(default)]
    pub audience_tiers: Vec<AudienceTier>,
    #[serde(default)]
    pub surface_mappings: Vec<SurfaceMapping>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Pack {
    pub revision: u64,
    pub deleted: bool,
    #[serde(flatten)]
    pub document: PackDocument,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PackSummary {
    pub id: String,
    pub revision: u64,
    pub description: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ContentRef {
    pub pack_id: String,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RevisionRef {
    pub pack_id: String,
    pub pack_revision: u64,
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DraftCreate {
    pub id: Option<String>,
    pub content: String,
    pub context: Option<ContentRef>,
    pub verb: Option<ContentRef>,
    pub author_kind: AuthorKind,
    #[serde(default = "empty_object")]
    pub provenance: serde_json::Value,
}

#[derive(
    Debug,
    Clone,
    Copy,
    Default,
    PartialEq,
    Eq,
    Serialize,
    Deserialize,
    clap::ValueEnum,
    schemars::JsonSchema,
)]
#[serde(rename_all = "kebab-case")]
pub enum AuthorKind {
    #[default]
    Human,
    Agent,
    Capture,
    Import,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DraftVersion {
    pub draft_id: String,
    pub version: u64,
    pub parent_version: Option<u64>,
    pub content: String,
    pub context: Option<RevisionRef>,
    pub verb: Option<RevisionRef>,
    pub author_kind: AuthorKind,
    pub provenance: serde_json::Value,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DraftTarget {
    pub draft_id: String,
    pub version: u64,
}

/// Transport-neutral input for atomically resolving assist material and
/// recording the source selection that will be sent to an attached agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AssistPreparation {
    pub surface: String,
    pub context: String,
    pub verb: String,
    pub pack: Option<String>,
    pub selection: String,
    pub surrounding_context: String,
    pub draft: Option<DraftTarget>,
}

/// Exact material and source identity frozen by [`Store::prepare_assist`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedAssist {
    pub pack_id: String,
    pub pack_revision: u64,
    pub context: RevisionRef,
    pub context_text: String,
    pub verb: RevisionRef,
    pub verb_instructions: String,
    pub source: DraftTarget,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttestationItemInput {
    pub draft_id: String,
    pub version: u64,
    #[serde(default)]
    pub decision: AttestationDecision,
    #[serde(default)]
    pub exceptions: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum AttestationDecision {
    #[default]
    Approved,
    Excepted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ApprovalVia {
    pub harness: String,
    pub session: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum ApprovalMethod {
    VerbalLgtm,
    Explicit,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AttestationInput {
    pub human_approved: bool,
    pub approved_by: String,
    pub via: ApprovalVia,
    pub method: ApprovalMethod,
    pub audience_tier: ContentRef,
    pub items: Vec<AttestationItemInput>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attestation {
    pub id: String,
    pub batch_id: String,
    pub human_approved: bool,
    pub approved_by: String,
    pub via: ApprovalVia,
    pub method: ApprovalMethod,
    pub audience_tier: RevisionRef,
    pub draft: DraftTarget,
    pub decision: AttestationDecision,
    pub exceptions: Vec<String>,
    pub created_at: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaptureInput {
    pub id: Option<String>,
    #[serde(default)]
    pub observation: CaptureObservation,
    pub parent_id: Option<String>,
    pub surface: String,
    pub url: String,
    pub content: String,
    pub draft: Option<DraftTarget>,
    #[serde(default = "empty_object")]
    pub metadata: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Capture {
    pub id: String,
    pub observation: CaptureObservation,
    pub parent_id: Option<String>,
    pub surface: String,
    pub url: String,
    pub content: String,
    pub draft: Option<DraftTarget>,
    pub metadata: serde_json::Value,
    pub captured_at: String,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum CaptureObservation {
    #[default]
    SubmitAttempt,
    SurfaceConfirmedPost,
}

fn empty_object() -> serde_json::Value {
    serde_json::json!({})
}

fn valid_key(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
}

fn text(value: &str, field: &str) -> std::result::Result<(), String> {
    if value.trim().is_empty() {
        Err(format!("{field} must not be empty"))
    } else {
        Ok(())
    }
}

fn unique<'a>(
    values: impl Iterator<Item = &'a str>,
    field: &str,
) -> std::result::Result<(), String> {
    let mut seen = BTreeSet::new();
    for value in values {
        if !valid_key(value) {
            return Err(format!("{field} contains invalid key `{value}`"));
        }
        if !seen.insert(value) {
            return Err(format!("{field} contains duplicate `{value}`"));
        }
    }
    Ok(())
}

impl PackDocument {
    pub fn validate(&self) -> std::result::Result<(), String> {
        if !valid_key(&self.id) {
            return Err("id must be 1-128 ASCII letters, digits, '.', '_' or '-'".into());
        }
        text(&self.description, "description")?;
        unique(self.contexts.iter().map(|v| v.name.as_str()), "contexts")?;
        unique(self.verbs.iter().map(|v| v.name.as_str()), "verbs")?;
        unique(
            self.audience_tiers.iter().map(|v| v.name.as_str()),
            "audience_tiers",
        )?;
        unique(
            self.surface_mappings.iter().map(|v| v.surface.as_str()),
            "surface_mappings",
        )?;
        let contexts: BTreeSet<_> = self.contexts.iter().map(|v| v.name.as_str()).collect();
        let tiers: BTreeSet<_> = self
            .audience_tiers
            .iter()
            .map(|v| v.name.as_str())
            .collect();
        for item in &self.contexts {
            text(&item.description, "context.description")?;
            text(&item.content, "context.content")?;
        }
        for item in &self.verbs {
            text(&item.description, "verb.description")?;
            text(&item.instructions, "verb.instructions")?;
            unique(
                item.context_bindings.iter().map(String::as_str),
                "verb.context_bindings",
            )?;
            for binding in &item.context_bindings {
                if !contexts.contains(binding.as_str()) {
                    return Err(format!(
                        "verb `{}` refers to missing context `{binding}`",
                        item.name
                    ));
                }
            }
            if let Some(length) = &item.constraints.length {
                if length.min.is_none() && length.max.is_none() {
                    return Err(format!(
                        "verb `{}` length constraint requires min or max",
                        item.name
                    ));
                }
                if length.min == Some(0) || length.max == Some(0) {
                    return Err(format!(
                        "verb `{}` length bounds must be greater than zero",
                        item.name
                    ));
                }
                if matches!((length.min, length.max), (Some(min), Some(max)) if min > max) {
                    return Err(format!(
                        "verb `{}` length min must not exceed max",
                        item.name
                    ));
                }
                if length.min.is_some_and(|v| v > i64::MAX as u64)
                    || length.max.is_some_and(|v| v > i64::MAX as u64)
                {
                    return Err(format!(
                        "verb `{}` length bounds exceed the supported maximum",
                        item.name
                    ));
                }
            }
        }
        for item in &self.audience_tiers {
            text(&item.description, "audience_tier.description")?;
            for requirement in &item.requirements {
                text(requirement, "audience_tier.requirements[]")?;
            }
        }
        for mapping in &self.surface_mappings {
            if !contexts.contains(mapping.context.as_str()) {
                return Err(format!(
                    "surface `{}` refers to missing context `{}`",
                    mapping.surface, mapping.context
                ));
            }
            if !tiers.contains(mapping.default_audience_tier.as_str()) {
                return Err(format!(
                    "surface `{}` refers to missing default audience tier `{}`",
                    mapping.surface, mapping.default_audience_tier
                ));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_schemas_reject_unknown_fields() {
        let json = r#"{"name":"tighten","description":"x","family":"transform-selection","instructions":"x","context_bindings":[],"constraints":{"length":null,"no_new_claims":true,"preserve_meaning":true,"judgment":true}}"#;
        assert!(serde_json::from_str::<VerbSpec>(json).is_err());
    }
}
