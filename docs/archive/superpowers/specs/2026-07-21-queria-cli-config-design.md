# queria-cli config design

> Status: REFERENCE — approved design for implementation  
> Last verified: 2026-07-21  
> Runtime truth when shipped: [`../../HANDOFF.md`](../../HANDOFF.md)  
> Related: index-here design, agent-setup MCP snippets, onboarding Daily path

## Problem

Laptop users must `export QUERIA_AGENT_TOKEN` / `QUERIA_EDGE_URL` every shell. Multi-account (work/personal) is painful. MCP install is manual paste from edge docs.

## Goals

1. Multi-profile user config on disk (token + edge + optional slug/mcp).
2. Human UX: **ratatui TUI only** — `queria-cli config` (no set/list subcommands).
3. Scripts/CI: env `QUERIA_*` or hand-edit TOML (not CLI flags).
4. `index-here` resolves credentials without required export.
5. MCP install from TUI for **droid, claude, cursor, codex** via live  
   `GET {edge}/api/v1/setup/mcp-snippet?client=`.

## Non-goals (v1)

- OS keychain / encryption at rest  
- Per-repo `.queria` as default  
- Hooks install in TUI  
- New shared crate  
- Silent full overwrite of multi-server MCP files  

## Locked decisions

| Topic | Choice |
|---|---|
| Storage | `~/.config/queria/config.toml` named profiles |
| Active | `active_profile` + `config use` + `--profile` / `QUERIA_PROFILE` |
| Code | Modules inside `queria-cli` only |
| Human UX | TUI only (`queria-cli config`) |
| MCP | Always fetch edge; no offline templates |
| Clients | droid, claude, cursor, codex |

## File format

Path order: `QUERIA_CONFIG` → `$XDG_CONFIG_HOME/queria/config.toml` → `~/.config/queria/config.toml`.

Unix: dir `0700`, file `0600`.

```toml
active_profile = "work"

[profiles.work]
edge_url = "https://queria.fjulian.id"
agent_token = "qria_…"
# mcp_url optional → {edge_url}/mcp
# project_slug optional
```

Profile name: `^[a-zA-Z0-9][a-zA-Z0-9_-]{0,63}$`.

## Credential resolve order

1. `--token-env` / `--edge-url-env` when used: read those **env names** (CI).  
2. Else env `QUERIA_AGENT_TOKEN`, `QUERIA_EDGE_URL`, `QUERIA_MCP_URL`, `QUERIA_PROJECT_SLUG`.  
3. Else profile from `--profile` or `QUERIA_PROFILE` or `active_profile`.  
4. Edge default only: `http://127.0.0.1:17674`; **token never defaulted**.

## CLI surface

```text
queria-cli [--profile NAME] config       # TUI only (TTY required)
queria-cli [--profile NAME] index-here …
```

`config` without TTY → error. No non-interactive config subcommands.
## Modules

| Module | Role |
|---|---|
| `config` | path, load/save, CRUD, redact |
| `credentials` | resolve into `ResolvedCredentials` |
| `mcp_install` | fetch snippet, backup, safe merge/write |
| `config_tui` | ratatui; calls shared functions |

## MCP rules

- Backup `*.queria-bak-<timestamp>` before write.  
- JSON: upsert `mcpServers.queria` only.  
- Codex TOML: upsert `[mcp_servers.queria]` only when parseable.  
- Droid/claude shell: print; execute only with `--yes`.  
- Unmergeable → fail + dry-run content; no silent full clobber.

## Tests

Path resolve, CRUD, redact, resolve order, JSON upsert, non-TTY bare config exit 2, `cargo test -p queria-cli`.

## Docs when shipping

Onboarding install: prefer `queria-cli config` then `index-here`; Daily MCP via `config mcp` / `config env` / export. HANDOFF residual until shipped.
