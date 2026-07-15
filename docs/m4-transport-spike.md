# M4 attached-agent transport spike

Measured on 2026-07-14 on the local arm64 macOS development host with OpenCode
`1.18.0+d6f1b08`. These are lifecycle and control-plane measurements; they do
not invoke a model, so model/provider latency is intentionally excluded.

| transport | cold lifecycle measurement | warm control measurement | session reuse |
| --- | ---: | ---: | --- |
| command process | 18.85 ms median, 22.26 ms p95 (30 runs) | not applicable | none unless the wrapper implements it |
| OpenCode HTTP | 443.57 ms to `/global/health` | 0.32 ms median, 0.68 ms p95 health RTT (30 requests) | native persistent sessions |
| OpenCode ACP | 502.46 ms median, 514.46 ms p95 initialize (10 processes) | not measured without creating a session | advertised `loadSession`, list, resume, fork, and close |

The command measurement launches a direct text-only fixture with a 4 KiB stdin
payload and candidate-only stdout. HTTP starts `opencode serve --pure` on
loopback and probes its documented health endpoint. ACP starts `opencode acp
--pure`, negotiates protocol version 1 over stdio, and records the initialize
response. Raw samples ranged from 15.07-32.56 ms for command launch,
0.24-3.15 ms for warm HTTP health, and 496.39-532.45 ms for ACP initialize.

## Lifecycle and permissions

- **Command:** one bounded child per assist request; simplest failure and
  cancellation boundary. prose sends text on stdin and accepts candidate text
  on stdout. The configured wrapper owns model choice and must enforce its own
  filesystem, shell, network, and posting restrictions. No session state crosses
  requests unless the wrapper deliberately adds it.
- **OpenCode HTTP:** one supervised server amortizes startup and exposes native
  persistent sessions. Its API is much broader than prose's text contract, so a
  future adapter must use loopback/password protection, a restricted OpenCode
  agent policy, explicit tool restrictions, bounded requests, and session
  cleanup. prose should not proxy the general OpenCode API to the extension.
- **ACP:** keeps a standard subprocess session and advertises session reuse. The
  client can decline filesystem and terminal callback capabilities, as this
  spike did, but OpenCode's own agent/tool permissions still need an explicit
  restricted policy. ACP requires request/cancellation/session handling beyond
  the command contract but avoids an HTTP service credential and port.

## Decision

Keep command transport as the M4 default. Its roughly 19 ms process overhead is
small compared with inference, its text boundary is explicit, and its lifecycle
is now bounded and reaped. HTTP is the strongest candidate when measured
multi-turn reuse materially improves end-to-end assist latency. ACP is preferable
when protocol portability and session lifecycle outweigh its integration cost.

Before changing the default, dogfood under projects#256 should measure complete
assist latency, permission prompts, session reuse hit rate, cancellation, and
backend failures with the same restricted model/agent configuration. No option
uses deprecated MCP Sampling.
