# mccboard

A control center for running several Claude Code accounts side by side: it sets them up, keeps them consistent, and shows how much of each account's usage is left.

## Language

**Account**:
A Claude subscription identity (claude.ai login), identified by its account UUID. Usage belongs to the account, whatever machine or surface it is used from.
_Avoid_: user, login, profile

**Account label**:
The short name and color the user gives an account ("A", blue). Used for display only, never for identity.
_Avoid_: alias, nickname

**Machine**:
A computer where instances live: the local machine, or a remote one reached over SSH.
_Avoid_: host, server, node

**Main machine**:
The machine running mccboard. Today a Windows laptop.

**Instance**:
A Claude Code config directory on a machine. Its account is observed, not owned: it can be logged out, expired, or switch to another account, and it stays the same instance.
_Avoid_: install, setup, environment

**Primary instance**:
The instance in Claude Code's default config directory on a machine (`~/.claude`). It holds the real copies of everything shared.
_Avoid_: main instance, default instance, source

**Linked instance**:
Any other instance on the same machine. It points at the primary instance's shared items instead of holding its own.
_Avoid_: secondary, child, clone

**Launcher**:
The command that starts Claude Code in a linked instance (`claude-b`). The primary instance keeps plain `claude`.
_Avoid_: alias, shim, wrapper

**Desktop profile**:
A Claude desktop app installation with its own data directory and its own sign-in, bound to one account by the user.
_Avoid_: desktop instance, app instance

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
