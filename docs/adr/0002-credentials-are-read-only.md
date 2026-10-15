# Credentials are read-only, and a token never leaves its machine

mccboard reads an instance's OAuth access token only to call the usage endpoint. It never refreshes the token, never writes the credentials file, and never runs `/logout`. Refreshing rotates the refresh token and logs Claude Code out. `/logout` revokes the token.

For a remote instance, the remote makes the call over SSH and only the result comes back, so a token never leaves its machine. The usage endpoint's budget is shared with Claude Code, so calls are rationed. The exact rules are the guardrails in `AGENTS.md`.

Reading the token at all is a deliberate exception: it's the only way to get fresh usage for an account no session is using. claude-swap, bozdemir/claude-usage-widget and Claude-Code-Usage-Monitor all read it the same way and never refresh it.
