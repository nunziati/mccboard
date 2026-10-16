# mccboard

A control center for running several Claude Code accounts side by side: it sets them up, keeps them consistent, and shows how much of each account's usage is left.

## Language

**Account**:
A Claude subscription identity (claude.ai login), identified by its account UUID. Usage belongs to the account, whatever machine or surface it is used from.
_Avoid_: user, login, profile

**Account label**:
The short name and color the user gives an account ("A", blue). Used for display only, never for identity. The account of the default desktop profile always has Claude's own orange, matching the icon mccboard leaves alone.
_Avoid_: alias, nickname

**Machine**:
A computer where instances live: the local machine, or a remote one reached over SSH.
_Avoid_: host, server, node

**Main machine**:
The machine running mccboard. Today a Windows laptop.

**Instance**:
A Claude Code config directory on a machine. Its account is observed, not owned: it can be logged out, have an expired token, or switch to another account, and it stays the same instance.
_Avoid_: install, setup, environment

**Expired token**:
An instance whose OAuth access token is past its expiry while its credentials are still there. Claude Code renews it the next time it runs in that instance, or when mccboard runs it there for that purpose (a token renewal); mccboard itself never does. It is still logged in: not logged in means no credentials, or ones Claude Code emptied on logout.
_Avoid_: login expired, logged out

**Primary instance**:
The instance in Claude Code's default config directory on a machine (`~/.claude`). It holds the real copies of everything shared.
_Avoid_: main instance, default instance, source

**Linked instance**:
Any other instance on the same machine. It points at the primary instance's shared items instead of holding its own.
_Avoid_: secondary, child, clone

**Launcher**:
The command that starts Claude Code in a linked instance (`claude-b`). The primary instance keeps plain `claude`. On Linux it is a shell alias; on Windows a copy of mccboard's shim program (`claude-b.exe`, ADR 0007).
_Avoid_: alias, shim, wrapper

**Desktop profile**:
A copy of the Claude desktop app running with its own data directory and its own sign-in, bound to one account by the user; at most one per account on a machine. Its account is observed like an instance's. It may also name the instance whose config directory its local Code sessions use. Every profile but the default one has its own taskbar button and an icon in its account's color.
_Avoid_: desktop instance, app instance, instance (an instance is a Claude Code config directory)

**Default desktop profile**:
The desktop profile in the app's own data directory, the one it uses when started normally. It keeps Claude's own icon and pairs with the primary instance.
_Avoid_: main profile, primary profile

### Sharing between instances

**Linked item**:
Something a linked instance uses from the primary instance through a link, so there is only one copy (`settings.json`, each personal skill).
_Avoid_: shared file, symlink

**Aligned item**:
Something each instance keeps its own copy of, which mccboard compares and brings in line (Global config keys, installed plugins).
_Avoid_: synced item, copied item

**Private item**:
Something that belongs to one instance and that mccboard never touches (credentials, session history, the account's synced skills).
_Avoid_: local item, excluded item

**Drift**:
Any difference between how an instance is and how its sharing says it should be: a broken link, a misaligned key, a missing plugin, a new unshared skill.
_Avoid_: inconsistency, mismatch

### Usage

**Usage window**:
A rolling period over which Anthropic limits an account's usage, with a utilization (0–100) and a reset time.
_Avoid_: limit, quota, rate limit

**Session window**:
The 5-hour usage window.
_Avoid_: 5h limit, session limit

**Weekly window**:
The 7-day usage window covering all models.
_Avoid_: 7d limit, weekly limit

**Model window**:
A 7-day usage window that applies to one model only.
_Avoid_: per-model limit, scoped limit

**Usage reading**:
All of an account's usage windows at one moment, with its source (endpoint or session) and the time it was read.
_Avoid_: snapshot, usage data, stats

**Stale**:
Said of a usage reading too old to trust as current; it is still shown, marked as such.
_Avoid_: expired, outdated
