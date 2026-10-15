# Usage comes from sessions first, the endpoint second

An account's usage reading comes from the freshest, least invasive source:
1. **Sessions first.** A statusline script that mccboard installs in each instance wraps the existing one (GSD, for example) and records the `rate_limits` Claude Code already reports. The data is official and costs no network calls.
2. **The endpoint otherwise.** `GET https://api.anthropic.com/api/oauth/usage`, called once per account, not per instance, preferring a local instance's token.

The base ships the endpoint only. The session source is the first feature after it, and it changes only where a reading comes from.

## Considered Options

- **Estimates from transcripts** (Claude-Code-Usage-Monitor): Anthropic doesn't publish per-plan token limits, so the percentages would be guesses.
- **claude.ai web cookies** (SlavomirDurej/claude-usage-widget): needs a spoofed browser user agent.
