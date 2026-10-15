# mccboard

**Multi Claude Code dashBoard**: a control center for several Claude Code instances and accounts. It includes:
- a widget docked in the OS taskbar, plus a tray icon;
- a dashboard;
- side-by-side Claude desktop app profiles, one per account;
- guided setup of remote machines over SSH.

Personal project of Giacomo.

- **Vocabulary:** `CONTEXT.md` is the glossary. Use its terms in code, issues and UI (account, instance, primary/linked instance, drift, usage reading…).
- **Why things are as they are:** `docs/adr/`. Read the relevant ADR before changing architecture, data sources, credential handling or how instances share files.
- **Project history and plans** live in the management repo `nunziati/claude-account-switcher-project` (Italian): useful for background, never needed to build.

## Architecture
- **One app on the main machine**; remote machines optional, over agentless SSH (ADR 0001).
- **Cargo workspace** (ADR 0004):
  - `crates/core`: domain, data sources, setup operations, token rules. No Tauri dependency.
  - `crates/cli`: command-line front end over the core.
  - `src-tauri`: thin binding of the core to Tauri commands, tray and windows.
  - `ui/`: Svelte.
  
  New behavior goes in the core first, then gets a CLI command and a UI.
- **Usage**: sessions first, endpoint second, once per account (ADR 0003).
- **Sharing between instances**: linked, aligned and private items (ADR 0005); Developer Mode for file links on Windows (ADR 0006).

## Token rules: hard guardrails
Credentials are **read-only** to mccboard. Claude Code owns them.
1. Read the OAuth access token from `<config dir>/.credentials.json`, and only read it. Refreshing it (any call to `oauth/token`) or writing the credentials file rotates the refresh token and logs Claude Code out. An expired token is shown as "expired" until Claude Code renews it.
2. **A token stays on its machine.** For a remote instance, the remote performs the endpoint call over SSH and returns only the result.
3. **The usage endpoint has a small budget shared with Claude Code**:
   - at most one request per 60 s per account;
   - after a 429 or an error, double the interval, up to 300 s;
   - show the last known values;
   - zero a window once its `resets_at` passes.
4. **Switch accounts with another config directory.** `/logout` revokes the token, so mccboard never runs or suggests it.
5. **Keep tokens and credentials out of output**: logs, errors, tests, files, UI.
6. mccboard's own files live in `~/.mccboard/`. `~/.claude` belongs to Claude Code.

## Stack
- **Tauri v2.** Core in Rust. OS-specific code (taskbar docking, Windows taskbar IDs, icons, launchers) lives in per-OS modules.
- **UI:** Svelte + TypeScript.
- **Platforms:** Windows is the target. Core, CLI and app also build and pass tests on Linux. macOS is out of scope for now.

## Conventions
- **Repo language:** English for code, comments, README and docs.
- **App language:** configurable, Italian and English. Every UI string goes through i18n.
- **Settings:** `~/.mccboard/config.toml` (hand-editable: machines, accounts, instances, desktop profiles, preferences). Data mccboard writes on its own goes under `~/.mccboard/state/`. Instances exist for mccboard only once registered; scanning only proposes them.
- **Tests:** test-first on the core. File operations are tested on real temporary directories, not mocks.
- **Workflow:** one issue, one branch, one PR. CI (Ubuntu + Windows) must be green, then Giacomo merges.

## Agent skills

### Issue tracker

GitHub Issues on `nunziati/mccboard`, via `gh` or the GitHub MCP tools. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
