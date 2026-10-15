# One app on the main machine; remotes over agentless SSH

mccboard is a single app on the main machine, which is also the dashboard. Remote machines and extra accounts are optional. mccboard reaches a remote through the system `ssh` and runs small scripts it carries along (POSIX shell, PowerShell later), the way Ansible or VS Code Remote-SSH do. Nothing runs permanently on a remote. We can't assume an always-on second machine exists, and a resident agent would need installing, updating and exposing on the network.

## Considered Options

- **Server on an always-on machine** (the first design, built around a mini PC): rejected, because that machine may not exist.
- **Resident agent on each remote** pushing data: more reactive, but needs install, updates and a network surface.
- **Synced folder** (Syncthing and similar): one more dependency, and delays.
- **Through Anthropic's servers** (Remote Control, claude.ai): no public API exposes this data.
