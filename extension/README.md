# prose browser extension

This unpacked Manifest V3 extension supports Chrome and Firefox on GitHub and
Linear. It builds its context menu from the active pack's verbs, asks prose to
resolve the surface mapping, sends assist requests through the localhost
service, and captures submitted text independently of backend availability.
Captures queue in extension-local storage while the service is unavailable and
retry on startup and the periodic refresh alarm.

The extension contains no voice modules, verbs, tiers, or model calls. Surface
IDs (`github-pr` and `linear`) are detection results; the active pack owns their
context and audience mappings.

Start `prose serve` on the default `127.0.0.1:37673`, then load this directory
as an unpacked/temporary extension. Assist requires `--agent-command`; submit
capture continues when the backend is missing or down.
