---
name: rootwhy
description: Use when a command needs root (sudo, pkexec, doas, su) — installing system packages, editing files in /etc or /usr, restarting system services, changing owners or permissions of system files — or when a direct sudo call was blocked.
---

# Root commands through rootwhy

Never call `sudo`, `pkexec`, `doas`, `run0` or `su` directly — the user could not see why, and a hook stops it.
Run the command through **rootwhy**: the user gets a dialog with your reason, the command split into coloured parts
and, on hover, what each part does (from the system's man pages), and types the password there. You never see it.

```bash
rootwhy -r "What changes and why, 1–2 sentences in the user's language." -- command args…
```

1. **Check it is installed:** `command -v rootwhy`. If missing, tell the user and point to
   https://github.com/DragonKing026/rootwhy#installation — do not install it yourself, and do not fall back to sudo.
2. **Is root really needed?** Prefer user-level alternatives (`--user` installs, `~/.local`, user systemd units).
3. **Tell the user first** that a password dialog is about to appear, in one sentence.
4. **Reason (`-r`, 10–300 characters)** — the user's language, understandable without this conversation: what will
   change and why, e.g. `Restart Ollama so it picks up the new model directory.` Not a restatement of the command.
5. **One dialog per task:** several steps → one script, `rootwhy -r "…" -- bash /path/to/script.sh`, so the user
   types the password once. Prefer absolute paths; the dialog warns when a script could be changed by someone other
   than root.
6. **Exit codes:** the command's own; `130` = the user cancelled → stop and ask, never retry on your own; `2` = usage
   error (missing reason, no graphical session); `127` = command not found.

`rootwhy --preview -r "…" -- command` shows the dialog without running anything. If the project defines its own
wrapper around rootwhy (e.g. one that logs elsewhere), use the wrapper.
