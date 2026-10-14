# mccboard

**Multi Claude Code dashBoard**: a control center for several Claude Code instances and accounts. It includes:
- a widget docked in the OS taskbar, plus a tray icon;
- a dashboard;
- side-by-side Claude desktop app profiles, one per account;
- guided setup of remote machines over SSH.

Personal project of Giacomo.

**Before any architectural change, or when a decision's "why" is unclear**, read the management repo `nunziati/claude-account-switcher-project` (written in Italian):
- `CONTEXT.md`: decisions D005 onward;
- `docs/2026-10-14-centro-di-controllo.md`: architecture and phases;
- `research/`: existing usage widgets, claude-swap, multi-claude, Claude Code internals.

## Architecture
- **One app, on the main machine** (today Windows). Everything mccboard runs lives in that app.
- **Remote machines are optional.** The app reaches them through the system `ssh`, running small scripts it carries along, so a remote holds only what guided setup created.
- **Multi-account is optional.** Domain model:
  - *account*: usage belongs to the account, across machines;
  - *instance*: account + config directory + machine;
  - *desktop profile*: a Claude desktop app instance with its own `--user-data-dir`.
- **Usage data**: session data first (statusline captured per instance), else `GET https://api.anthropic.com/api/oauth/usage`, once per account.

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
- **Tauri v2.** Core in Rust. OS-specific code (taskbar docking, Windows taskbar IDs, icons) lives in per-OS modules.
- **UI:** Svelte + TypeScript.

## Conventions
- **Repo language:** English for code, comments, README and docs.
- **App language:** configurable, Italian and English. Every UI string goes through i18n.
- **Settings:** persisted in `~/.mccboard/`. They survive closing the dashboard and restarting the app.

## Agent skills

### Issue tracker

GitHub Issues on `nunziati/mccboard`, via `gh` or the GitHub MCP tools. See `docs/agents/issue-tracker.md`.

### Triage labels

Default vocabulary: `needs-triage`, `needs-info`, `ready-for-agent`, `ready-for-human`, `wontfix`. See `docs/agents/triage-labels.md`.

### Domain docs

Single-context: `CONTEXT.md` + `docs/adr/` at the repo root. See `docs/agents/domain.md`.
