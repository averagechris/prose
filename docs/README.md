# prose docs

Design and reference documentation. The umbrella tracker is canonical for
history and status: [#246 (intention and non-negotiables)](https://todo.sr.ht/~averagechris/projects/246),
[#247 (architecture, milestones; status lives in its comments)](https://todo.sr.ht/~averagechris/projects/247).

- [design.md](design.md): north star, invariants, architecture shape, and
  milestone map. Read this before designing or building anything.
- [m1.md](m1.md): M1 library, SQLite schema, strict JSON contracts, CLI, and
  machine-output reference.
- [m2.md](m2.md): shared application operations, MCP stdio tools, and host
  pointer rendering.
- [m3.md](m3.md): loopback HTTP service, attached-agent command transport,
  immutable captures, and the GitHub/Linear browser extension.
- [m4.md](m4.md): native pack provenance, capture observations, service and
  browser hardening, and extension packaging.
- [m4-transport-spike.md](m4-transport-spike.md): measured command, OpenCode
  HTTP, and ACP lifecycle/permission comparison.
- [post-m4.md](post-m4.md): refined evidence-reader, harness/onboarding,
  authenticated-remote, phone-client, and privacy-bounded mining architecture.
- `pages/`: static HTML stubs rendered on the fleet site (downloads,
  overview, examples, changelog). Not design docs.
