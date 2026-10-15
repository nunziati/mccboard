# Windows requires Developer Mode for file links

`settings.json` and `CLAUDE.md` need a real symbolic link: Claude Code's atomic save breaks a hard link. On Windows, creating a file symlink without elevation requires Developer Mode. mccboard checks for it and explains how to enable it, and won't link without it. Directories use junctions, which need no privilege.

## Considered Options

- **A UAC prompt per link set:** works without Developer Mode, but asks for admin rights repeatedly.
- **Copies kept in sync by mccboard:** needs no privilege, but means two files to reconcile, with conflicts.

Either can come back if someone without Developer Mode needs mccboard.
