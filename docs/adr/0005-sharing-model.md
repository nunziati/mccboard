# Sharing between instances: linked, aligned, private

Each account gets its own instance (config directory) via `CLAUDE_CONFIG_DIR`, the multi-account setup Claude Code documents. A linked instance shares the primary instance's setup in three ways.

- **Linked** (one copy, reached through a link): `settings.json`, `CLAUDE.md`, `agents/`, `hooks/`, `output-styles/`, `scripts/`, and **each personal skill folder separately**, never `skills/` as a whole.
- **Aligned** (each instance has its own copy; mccboard compares them and brings them in line): the "Global config" keys of `.claude.json`, and installed plugins and marketplaces, reinstalled with `claude plugin install`.
- **Private** (mccboard never touches it): credentials, the rest of `.claude.json`, `projects/`, history, `skills/synced/`, `plugins/`.

Items not on these lists, GSD's files for example, are offered once, and the answer is remembered.

Why not link `skills/` and `plugins/` whole, as a first manual setup did:
- **Skills.** Claude Code downloads each account's claude.ai skills into `skills/synced/`, and no setting moves that folder (anthropics/claude-code#101422).
- **Plugins.** A shared `plugins/` fails marketplace validation from both sides, because `installLocation` is compared by literal path (anthropics/claude-code#82272, reproduced by a maintainer).

`CLAUDE_CODE_PLUGIN_CACHE_DIR` might allow sharing plugins later. It is untested.

## Consequences

- A `settings.json` link must be a single hop pointing straight at the real file. Claude Code saves it atomically, resolving the link one hop (anthropics/claude-code#78162).
- When working under the home directory, the primary's `settings.json` also loads as project settings in a linked instance (anthropics/claude-code#80791), so its hooks could run twice. The 2026-10 field test did not observe this: each hook ran once per start, from the home folder and from a subfolder. The doctor doesn't watch for it.
