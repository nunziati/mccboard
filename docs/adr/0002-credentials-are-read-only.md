# Credentials are read-only, and a token never leaves its machine

mccboard reads an instance's OAuth access token only to call the usage endpoint. It never refreshes the token, never writes the credentials file, and never runs `/logout`. Refreshing rotates the refresh token and logs Claude Code out. `/logout` revokes the token.

For a remote instance, the remote makes the call over SSH and only the result comes back, so a token never leaves its machine. The usage endpoint's budget is shared with Claude Code, so calls are rationed. The exact rules are the guardrails in `AGENTS.md`.

Reading the token at all is a deliberate exception: it's the only way to get fresh usage for an account no session is using. claude-swap, bozdemir/claude-usage-widget and Claude-Code-Usage-Monitor all read it the same way and never refresh it.

## Amendment (#30): Claude Code may be asked to renew an expired token

An access token lasts about 8 hours. With no session in an account's instances it expires, and the account nobody is using, the one you'd want to switch to, has no fresh usage. So mccboard may run **Claude Code** in that instance and let it renew its own token: still never `oauth/token`, never a write to the credentials file.

- The command is `claude mcp get mccboard-renewal-no-such-server`, run with the instance's config directory (#51). Before looking the server up, Claude Code loads the account's claude.ai connectors, and for that it waits until it has renewed an expired token and rewritten its own credentials. Then it finds no such server and exits with 1, the expected end: no MCP server is started, no model call, no session window (checked by hand with Claude Code 2.1.301, from an ordinary terminal). mccboard discards its output unread and only reads the token again.
- It runs hidden, with no input, a one-minute timeout, and in an empty folder of mccboard's own (`~/.mccboard/state/renewal/`), so no project's settings, hooks or `CLAUDE.md` apply.
- Only when a usage reading is due and every token of the account has expired; once per account, in the instance whose token expired last. At most once per instance every 15 minutes, doubling up to 6 hours after a failure, reset by a success; reserved in mccboard's state file before it runs, so the app and the CLI don't both run it.
- A preference in `config.toml` (`renew_expired_tokens`, on by default) turns it off: an expired token then stays expired until Claude Code runs on its own, as before.
- A running session keeps its token valid, so an expired token means no session runs there: no session guard is needed.
- For a remote instance the same command would run over SSH on the remote, so the token still never leaves its machine.

Considered and set aside:
- `claude auth status`, the first choice (#30). It only reads the login state: Claude Code starts a token refresh at startup without waiting for it, and `auth status` exits in about 300 ms, usually before it is done, leaving the token expired. The first manual check passed only because it ran inside a Claude desktop app session, whose variables slowed startup enough for the refresh to finish.
- A one-shot prompt (`claude -p .`, as Claude-Code-Usage-Monitor does): it renews the token too, but counts as usage and may start the session window. The fallback if `mcp get` stops renewing.
- Both depend on how Claude Code works inside; so does `mcp get`. A renewal that keeps failing shows up as such (still expired, next try), and the preference turns it off.
