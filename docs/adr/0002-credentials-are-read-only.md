# Credentials are read-only, and a token never leaves its machine

mccboard reads an instance's OAuth access token only to call the usage endpoint. It never refreshes the token, never writes the credentials file, and never runs `/logout`. Refreshing rotates the refresh token and logs Claude Code out. `/logout` revokes the token.

For a remote instance, the remote makes the call over SSH and only the result comes back, so a token never leaves its machine. The usage endpoint's budget is shared with Claude Code, so calls are rationed. The exact rules are the guardrails in `AGENTS.md`.

Reading the token at all is a deliberate exception: it's the only way to get fresh usage for an account no session is using. claude-swap, bozdemir/claude-usage-widget and Claude-Code-Usage-Monitor all read it the same way and never refresh it.

## Amendment (#30): Claude Code may be asked to renew an expired token

An access token lasts about 8 hours. With no session in an account's instances it expires, and the account nobody is using, the one you'd want to switch to, has no fresh usage. So mccboard may run **Claude Code** in that instance and let it renew its own token: still never `oauth/token`, never a write to the credentials file.

- The command is `claude auth status`, run with the instance's config directory. With an expired token Claude Code renews it and rewrites its own credentials; it makes no model call and starts no session window (checked by hand with Claude Code 2.1.301). Its output names the account, so mccboard discards it unread and only reads the token again.
- It runs hidden, with no input, a one-minute timeout, and in an empty folder of mccboard's own (`~/.mccboard/state/renewal/`), so no project's settings, hooks or `CLAUDE.md` apply.
- Only when a usage reading is due and every token of the account has expired; once per account, in the instance whose token expired last. At most once per instance every 15 minutes, doubling up to 6 hours after a failure, reset by a success; reserved in mccboard's state file before it runs, so the app and the CLI don't both run it.
- A preference in `config.toml` (`renew_expired_tokens`, on by default) turns it off: an expired token then stays expired until Claude Code runs on its own, as before.
- A running session keeps its token valid, so an expired token means no session runs there: no session guard is needed.
- For a remote instance the same command would run over SSH on the remote, so the token still never leaves its machine.

Considered and set aside: a one-shot prompt (`claude -p .`, as Claude-Code-Usage-Monitor does) also renews the token, but counts as usage and may start the session window.
