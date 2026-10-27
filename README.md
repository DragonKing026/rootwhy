<p align="center"><img src="logo.svg" width="112" alt="rootwhy logo"></p>

# rootwhy

**sudo with a password dialog that says _why_ — and explains every part of the command.**

[Polski](README.pl.md)

When an AI coding agent (or a script) needs root, it usually ends up in front of you as a bare password prompt or
a one-line command you have to decode. `rootwhy` shows a dialog with:

- the **reason**, written by whoever asks (e.g. the agent),
- the **command split into lines and coloured** — command, options, values, paths,
- an **explanation of every part on hover**: what the requester claims, a built-in plain-language description,
  facts checked right now (does the file exist, who owns it, permissions, size, is the destination about to be
  overwritten) and the option's description from `man`,
- the working directory and the process chain that asked.

The password goes from the dialog straight to `sudo`. The caller never sees it.

![rootwhy dialog](docs/dialog-en.png)

Hovering over `755`, for example, shows *permissions rwxr-xr-x: owner read, write, execute; group read, execute;
others read, execute*. Hovering over the destination shows whether it exists and what lives in `/usr/local/bin`.

## Usage

```bash
rootwhy -r "Restart Ollama after changing its service file." systemctl restart ollama

rootwhy -r "Install the nightly backup script system-wide so the timer can run it as root." \
        -e "755=the script must be executable" \
        -e "nightly-backup.sh=new version with the fixed target path" \
        -- install -m 755 -o root -g root ./build/nightly-backup.sh /usr/local/bin/nightly-backup.sh
```

| Option | |
| --- | --- |
| `-r, --reason TEXT` | why the command is needed, 10–300 characters (required) |
| `-e, --explain FRAGMENT=TEXT` | explanation shown on hover over the matching part; repeatable. Matches a whole argument, an option name or the end of a path. Explanations that match nothing are listed under the command |
| `--log-dir DIR` | log directory (default `$XDG_STATE_HOME/rootwhy`, i.e. `~/.local/state/rootwhy`; also `ROOTWHY_LOG_DIR`) |
| `--preview` | show the dialog only, run nothing |

Exit code: the command's own, `130` when cancelled, `2` for usage errors. A wrong password lets `sudo` ask again
(up to 3 times), and the dialog says so.

### For AI agents

Tell the agent to use `rootwhy` instead of `sudo`, for example in `CLAUDE.md` / `AGENTS.md`:

```markdown
Commands that need root: only through `rootwhy -r "What and why, 1–2 sentences." [-e PART=WHY …] -- command …`,
never `sudo` directly. Tell the user a password dialog is about to appear. Exit code 130 means they cancelled —
do not retry without asking.
```

## Security model

- **Every call asks for the password** (`sudo -k`): no reuse of an earlier approval.
- **Askpass mode only under sudo.** `rootwhy` is its own `SUDO_ASKPASS`; in that mode it refuses to show the dialog
  unless its parent process is `sudo`, so an inherited environment cannot turn it into a password prompt that prints
  your password to whoever started it.
- **What you see is what runs.** The dialog shows the complete argument list — nothing is shortened. Arguments are
  shell-quoted, and invisible or bidirectional-control characters are shown as `$'‮'` escapes.
- **Nothing runs before you agree.** Explanations come from a built-in table, `man` (which never executes the
  command), `stat` of paths and `systemctl show` for unit descriptions.
- **Claims are marked as claims.** The reason and `-e` texts come from the requester and are labelled *claim, not
  verified*; facts checked by rootwhy are labelled *Checked now*. A dishonest requester can lie in the reason —
  the command itself is what you approve.
- **Log without passwords:** date, reason, command and exit code, one line each, written before the dialog opens.

Limits: `rootwhy` cannot tell what a script will do when it runs — only what file it is and who can change it (it
warns when the program or script is not owned by root or is writable by others). Passwordless `sudo` rules
(`NOPASSWD`) or PAM methods such as a fingerprint reader skip the askpass dialog entirely.

## Installation

Requirements: Linux, `sudo`, Python 3.9+, and for the full dialog GTK 4 with PyGObject. Without GTK 4 it falls back to
`kdialog` or `zenity` (reason and command split into lines, no colours or hover).

```bash
# Fedora
sudo dnf install python3-gobject gtk4
# Debian, Ubuntu, Kubuntu
sudo apt install python3-gi gir1.2-gtk-4.0

git clone https://github.com/DragonKing026/rootwhy.git ~/.local/share/rootwhy
ln -s ~/.local/share/rootwhy/rootwhy ~/.local/bin/rootwhy
rootwhy --version
```

Works on KDE Plasma and GNOME, Wayland and X11. Update: `git -C ~/.local/share/rootwhy pull`. Uninstall: remove the
link and the directory (and `~/.local/state/rootwhy` for the log).

## Language

The interface follows `LANG` / `LC_MESSAGES`: Polish for `pl_*`, English otherwise. Force it with `ROOTWHY_LANG=en`
or `ROOTWHY_LANG=pl`. Descriptions taken from `man` are in whatever language your man pages are.

## Development

```bash
python3 -m unittest discover tests        # command model, explanations, translations (no GTK, no sudo)
./rootwhy --preview -r "Testing the dialog." -- install -m 644 a /etc/b
./rootwhy --screenshot out.png -r "Testing the dialog." -- systemctl restart foo   # renders the window to PNG
```

The program is a single file on purpose — it receives your password, so it should be easy to read in one sitting.
Ideas and fixes for the built-in descriptions (`COMMANDS`, `_O`, `SUBCOMMANDS`) are welcome.

## Related projects

- [Salkcid/approve](https://github.com/Salkcid/approve) + [askpass](https://github.com/Salkcid/askpass) — Rust/GTK 4
  review dialog with a reason and process snapshot (Wayland only).
- [crypdick/sudoplz](https://github.com/crypdick/sudoplz) — approve agents' sudo with an SSH-key-encrypted password.

## License

[GPL-3.0-or-later](LICENSE). You may use, change and sell it, but anything you distribute that is based on it must
stay under the same license, with source code.
