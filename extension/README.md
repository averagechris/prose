# prose browser extension

This Manifest V3 extension supports conservative GitHub pull-request and
Linear adapters. It builds its context menu from service-provided verbs, asks
prose to resolve the surface mapping, sends assist requests through the
localhost service, and records exact submit attempts independently of backend
availability. It contains no voice modules, built-in verbs, tiers, personal
material, model calls, or content judgment.

GitHub is identified as `github-pr` only on
`/<owner>/<repository>/pull/<number>` routes. Native forms must contain exactly
one recognized PR editor. Linear's non-form fallback requires explicit
composer, Lexical-editor, and submit-control DOM markers. Generic labels and
buttons are ignored. These host DOM details can change; the deterministic tests
cover adapter contracts, but release smoke testing against current GitHub and
Linear pages is still required. False negatives are preferred to capturing an
unrelated editor. Assist selection follows those same adapters: GitHub search
fields and arbitrary contenteditables are not eligible, and Linear selection
must belong to the single recognized editor in an explicit composer. Assist
surrounding context is capped at 8,000 characters for both native and
contenteditable editors.

The extension emits stable-ID `submit-attempt` events only. It does **not** emit
`surface-confirmed-post`: no conservative exact-content confirmation adapter is
currently claimed for either host. A prevented or failed submission therefore
remains only an attempt. Content is not trimmed. A draft reference is attached
only when the exact editor snapshot still equals the exact inserted candidate;
human edits, removal, and SPA navigation invalidate it.

Captures persist in extension-local storage before delivery. Storage mutations
are serialized separately from delivery, so a stalled request cannot prevent a
later submit attempt from reaching durable local storage. Requests time out
after ten seconds. Network errors, timeouts,
429s, and 5xx responses retry on startup and a one-minute alarm. Permanent 4xx
responses are removed to a bounded dead-letter diagnostic (ID, status, and
timestamp only) so later captures continue. The five-minute alarm refreshes
service-provided context menus.

Capture message acknowledgement uses Chrome's callback/keepalive contract in
service workers and Firefox's Promise response contract in MV3 background
scripts. It acknowledges durable local persistence, not network delivery.

## Build and test

The package version comes from the root `Cargo.toml`; the manifest templates do
not duplicate it.

```sh
npm --prefix extension test
npm --prefix extension run check
node extension/scripts/build.mjs chrome ./dist/chrome
node extension/scripts/build.mjs firefox ./dist/firefox

nix build .#prose-extension-chrome
nix build .#prose-extension-firefox
```

Each Nix output contains `unpacked/` and a browser-named zip. The compatibility
attribute `.#prose-extension` points to the Chrome package.

Start `prose serve` on `127.0.0.1:37673`. Load the generated Chrome directory
with **Load unpacked** or the generated Firefox directory with
`about:debugging` → **This Firefox** → **Load Temporary Add-on**. Assist also
requires `--agent-command`; attempt capture continues while that backend or the
service is down.

## Signing and installation handoff

No signing credentials belong in this repository or the Nix build. For a
release, build the browser-specific zip reproducibly, inspect its generated
manifest/version, then submit the Chrome zip through the Chrome Web Store
publisher workflow and the Firefox zip through Mozilla Add-ons (`web-ext sign`
may be used in a credentialed release environment). Store resulting signed
artifacts in the release system, not in this working copy. Enterprise installs
may instead distribute the signed artifacts through browser policy.
