# Post-M4 architecture

This document refines the work after M4 without authorizing implementation or
future schema changes. M2-M4 hands-on acceptance is deferred to
[projects#257](https://todo.sr.ht/~averagechris/projects/257); supervised local
rollout remains owned by
[projects#256](https://todo.sr.ht/~averagechris/projects/256).

## Boundaries that remain fixed

- prose remains a model-free broker. Readers may project and diff evidence, but
  agents interpret it.
- One store belongs to one sovereign user. Supporting more users means making it
  easy for each person to own a store, not turning one prose process into a
  multi-tenant service.
- Posting integrations remain harnesses. prose records exact drafts and reported
  human approval; it does not acquire Slack, Telegram, GitHub, or email sending
  authority.
- Remote access is a client/server relation to one authoritative store, not
  SQLite replication or multi-master synchronization.
- Raw historical work evidence stays outside source control, releases, packs,
  and ordinary prose storage. Only reviewed derived material enters a pack.
- No future relation is inferred from content equality. Missing links stay
  missing.

## Dependency order

The next work should proceed in four independently useful milestones:

1. **M5: evidence reader and audit export.** Make recorded facts inspectable
   without adding judgment.
2. **M6: harness and pack-lifecycle contracts.** Prove that non-browser clients
   and users other than Chris can use the same primitives without special cases.
3. **M7: authenticated remote serve and phone client.** Extend the boundary only
   after local QA and a threat-model review.
4. **M8: privacy-bounded historical refinement.** Use private evidence to propose
   pack changes through the existing draft/approval/install loop.

M5 and architecture-only portions of M6 can be designed before dogfood, but M5's
public operation and export contract waits for projects#257 to confirm that real
evidence can reconstruct the intended workflows. M7 must not ship before that QA
confirms the local service boundary. M8 can inventory sources earlier, but must
not ingest work data before its privacy plan is approved.

## M5: evidence reader and audit export

### Read model

The first reader may project an illustrative `authoring episode` from existing
immutable records:

- one draft and its ordered versions;
- the exact approval attestations attached to versions;
- capture attempts and confirmed posts that carry an exact draft reference;
- context, verb, tier, and pack-item origins already referenced by those records.

The reader does not attach an unreferenced capture by comparing text. It reports
that evidence as unlinked. Multiple approvals or captures remain multiple facts;
the reader does not choose the "real" one.

Useful deterministic projections are:

- metadata-only timelines;
- exact before/after version pairs;
- a stable textual diff between selected versions;
- machine-readable JSON suitable for an attached agent;
- completeness facts such as whether native links reach an approval, submit
  attempt, confirmed post, or exact draft. Names and response shapes wait for the
  M5 data-model review; confirmed posts may reach a draft through their parent
  attempt rather than a direct reference.

There is no voice score, quality score, approval inference, or recommendation in
prose. An agent may consume the projection and propose a new draft; the human
still approves that exact version before installation.

### Privacy defaults

Evidence commands default to a safe metadata allowlist: prose-generated IDs,
versions, typed event kinds, timestamps, and booleans describing native-link
presence. URLs, arbitrary provenance/metadata, source labels and URIs, approver
or platform identities, and other user-provided strings are sensitive metadata
and are omitted by default. Returning sensitive metadata requires a separate
explicit option; returning draft, capture, or diff content requires an explicit
content option. Output goes only to the requested local transport; prose adds no
telemetry, background upload, or content-bearing diagnostics. Filters include
draft ID, time range, context, verb, surface, and evidence completeness so a
harness need not bulk-export the store.

### Audit export

An audit export is an explicit, bounded evidence document, not a database backup
or pack format. It records schema/version metadata, exact IDs, links, and
explicitly selected sensitive metadata or content. File export creates a private
file, refuses accidental overwrite, and cleans up partial failures. stdout and
MCP export are explicit disclosure boundaries rather than safe defaults. Import
is not part of M5: an exported approval must not silently become trusted approval
in another sovereign store.

### Acceptance

- After projects#257 evidence QA, CLI and MCP call one typed reader operation and
  return equivalent facts.
- Every link is backed by a native key already present in the store.
- Metadata-only is the default; content inclusion is explicit and testable.
- Unlinked captures remain visible without being guessed into episodes.
- Golden tests show deterministic ordering and diffs.
- The reader remains useful when no attached agent is configured.

## M6: harness and pack-lifecycle contracts

### Messaging-agent harness

A messaging integration is a separate harness process:

```text
private user-to-agent channel -> harness -> prose CLI/MCP -> agent runtime
                               -> exact draft -> explicit human approval
                               -> attestation -> harness sends through channel API
```

The private agent channel is not itself the destination audience. Incoming text
becomes a draft version only when the harness explicitly records it. Permanent
prose provenance excludes message text, usernames, channel names, internal URLs,
raw platform event IDs, and conversation identities. If retry correlation is
needed, the harness may record a keyed, non-reversible operation value; any
reversible mapping remains in the harness's private retention-bounded store.
prose does not retain the surrounding conversation by default.

Approval must identify one exact draft version. Reactions, silence, delivery
receipts, agent output, and ambiguous replies never count. The harness may accept
an explicit command or reply as approval, record the attestation, and only then
invoke its own posting capability. M6 does not assume prose-level attestation
idempotency: the reference harness keeps a private durable operation ledger and
reconciles prose state before retrying draft or attestation writes. Adding native
idempotency for those records would require a separately approved M6 data model.

The harness owns channel credentials and posting permissions. prose owns no
Telegram, Signal, Slack, or email credential and exposes no generic `send` tool.

### Onboarding another user

One user gets one private store and an empty active-pack state. Setup may explain
or import a separately distributed starter pack, but never auto-installs verbs,
tiers, mappings, examples, or Chris material. A useful onboarding flow is:

1. create/open a private store and report its location;
2. inventory available external pack artifacts without importing them;
3. explicitly import or create a pack;
4. choose the active pack;
5. map only the surfaces the user elects to support;
6. verify context, verbs, tier, origins, and backup expectations;
7. render host pointers that continue to fetch live material.

Pack edits use the existing external-import or approved-draft paths. Rollback is
a new append-only revision derived from selected old material, never movement of
an invisible mutable HEAD.

Ordinary pack export is material-only. Importing it into another store honestly
creates external-import origins; it does not transplant approval authority.
Portable approval provenance would require a separately designed audit bundle
with identity and trust semantics, and is out of M6.

### Acceptance

- A reference harness can complete draft -> explicit approval -> attestation
  without prose gaining posting authority.
- The reference harness's durable operation ledger prevents duplicate records or
  sends without placing raw platform identifiers in prose.
- Setup succeeds from an empty store with no built-in material.
- Another user can inspect where every installed item came from.
- Docs and errors never assume Chris-specific pack IDs, tiers, or mappings.

## M7: authenticated remote serve and phone client

### Network shape

Remote serve is opt-in and separate from the existing loopback browser router.
It remains one authoritative store with remote clients; there is no SQLite file
sharing, replica, CRDT, or offline multi-master merge.

Non-loopback startup requires all of:

- HTTPS at the process or an explicitly configured trusted reverse proxy whose
  backend is unreachable directly, with forwarding headers trusted only from
  configured proxy addresses and an authenticated proxy hop where applicable;
- rolled prose authentication rather than tailnet membership alone;
- an explicit remote listener and public base URL;
- a narrow remote route set with per-client scopes;
- durable audit identity for every write.

The loopback extension endpoint stays local and does not inherit remote routes or
credentials.

### Pairing and credentials

The sovereign user starts pairing locally with a short-lived, single-use proof.
Completing pairing creates a named client and returns a high-entropy secret once.
The implementation uses established token, password-hashing, cookie, and CSRF
libraries; prose designs no custom cryptography. Stored credentials must be
non-recoverable, scoped, expirable where practical, rotatable, and immediately
revocable. Exact tables and event shapes wait for the M7 threat model and schema
review.

Initial scopes should be narrower than domain nouns:

- read served pack material;
- create/append drafts;
- read exact drafts needed for approval;
- request assist;
- record captures, only for clients that actually observe a surface.

Agent and harness credentials never receive human-approval authority. Remote
approval is a distinct human-session capability issued only to the paired phone
client. If a future remote harness reports an approval witnessed elsewhere, that
is an auditable report under a separately defined trust contract, not authority
for the agent client to approve.

Pack administration, origin rewriting, database export, and client management
remain local-only until a concrete need justifies remote exposure.

### Phone approval client

The phone client is a thin web client, not an agent. It may show an exact draft,
its version, audience tier, provenance summary, and a digest useful for detecting
stale UI state. Approval uses a fresh review challenge bound to the authenticated
human session, exact displayed draft ID/version and tier, and one explicit human
action. A stale, changed, replayed, or expired challenge fails closed and must be
reviewed again.

Use secure, HTTP-only, same-site cookies after pairing for the browser session;
do not leave a durable bearer token in JavaScript storage. State-changing routes
require CSRF protection and origin checks in addition to authentication. The
client cannot approve in the background, infer approval from navigation, or post
to the destination surface.

### Acceptance

- Non-loopback serve refuses to start without the complete secure configuration.
- Authentication, scope checks, revocation, CSRF, and rate limits are enforced at
  the shared service boundary and covered by negative tests.
- Secrets, draft content, and bearer material never enter logs or health output.
- The phone can approve only the exact version displayed to the user.
- Capture and local CLI/MCP remain available during remote-client or agent
  failures.
- A threat-model review signs off before implementation leaves localhost.

## M8: privacy-bounded historical refinement

Historical mining is an attached-agent workflow, not a prose-owned crawler or
model. The source adapter first produces a dry-run inventory using explicit
repository/organization, date, context, and visibility allowlists. Normal
collection is hard-limited to material authored by the sovereign user and
excludes teammate-authored bodies and surrounding proprietary code. Any
third-party text or context requires a separate explicit privacy decision.
Collection proceeds only after that inventory and retention plan are approved.

Raw work reviews and messages remain in a private temporary workspace with
restrictive permissions, exclusion from repository paths, backup/sync, indexing,
tracing, and ordinary agent logs, and encryption at rest whenever retained.
Retention is bounded; cleanup fails closed and is recoverable after crashes. Raw
material is not written to the prose database, pack exports, traces, fixtures, or
commits. The workflow supports redaction, source deletion, and an audit of what
was read. Slack/DM collection remains disabled until company policy, teammate
privacy, retention, and export mechanics receive a separate explicit decision.

An agent may turn private evidence into independently reviewable candidate
principles or context-module changes. Evidence-to-candidate trace maps stay only
in the private temporary workspace. Only reviewed derived text enters prose as a
draft; if local correlation is essential, it uses a non-reversible identifier
with no source detail and is excluded from ordinary exports. Each principle is
explicitly approved or rejected before accepted principles are combined. Only
the reviewed exact combined version is attested and installed through native
approved-draft provenance. Public examples must be synthetic, already public,
personal, or separately approved.

### Acceptance

- Dry-run inventory precedes collection and names every allowlist/filter.
- Raw evidence cannot enter normal repository, release, pack-export, or prose
  database paths.
- Normal collection proves sovereign-user authorship and excludes teammate text
  and proprietary surrounding context.
- Derived output is reviewable separately from raw evidence and checked for
  accidental disclosure.
- Each candidate principle receives an explicit decision before combination.
- No guideline changes without an exact human attestation and native install
  link.
- Deletion and retention behavior are tested before work-source mining.

## Deferred questions

The following require evidence rather than architectural guessing:

- whether HTTP or ACP session reuse beats command transport end to end;
- which messaging platform is worth a first reference harness;
- whether confirmed-post DOM evidence becomes stable enough to enable;
- whether portable audit bundles are useful enough to justify identity/trust
  machinery;
- whether remote assist belongs in the first phone client;
- which historical repositories and date ranges pass the privacy review.

None of these questions should create a schema migration until its milestone has
a signed-off data model on paper.
