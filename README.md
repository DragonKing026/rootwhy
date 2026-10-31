<p align="center"><img src="logo.svg" width="112" alt="rootwhy logo"></p>

# rootwhy

[![test](https://github.com/DragonKing026/rootwhy/actions/workflows/test.yml/badge.svg)](https://github.com/DragonKing026/rootwhy/actions/workflows/test.yml)
[![CodeQL](https://github.com/DragonKing026/rootwhy/actions/workflows/codeql.yml/badge.svg)](https://github.com/DragonKing026/rootwhy/actions/workflows/codeql.yml)
[![security](https://github.com/DragonKing026/rootwhy/actions/workflows/security.yml/badge.svg)](https://github.com/DragonKing026/rootwhy/actions/workflows/security.yml)
[![release](https://github.com/DragonKing026/rootwhy/actions/workflows/release.yml/badge.svg)](https://github.com/DragonKing026/rootwhy/actions/workflows/release.yml)
[![OpenSSF Scorecard](https://api.scorecard.dev/projects/github.com/DragonKing026/rootwhy/badge)](https://scorecard.dev/viewer/?uri=github.com/DragonKing026/rootwhy)

**sudo with a password dialog that says _why_ — and shows what every part of the command does.**

[Polski](README.pl.md)

When an AI coding agent (or a script) needs root, it usually ends up in front of you as a bare password prompt or
a one-line command you have to decode. `rootwhy` shows a dialog with:

- the **reason**, written by whoever asks (e.g. the agent),
- the **command split into lines and coloured** — command, options, values, paths,
- **on hover, what each part does — from your system's own manual** (`man`, in your language when translated),
  plus a ⚠ warning when something deserves attention: the destination file will be replaced, the program or script
  could be changed by someone other than root, a path does not exist, a setuid mode, an unknown user,
- the working directory and the programs the request came through (e.g. `code → claude → zsh`).

Nothing is invented: rootwhy has no description table of its own and does not take explanations from the
requester. A part the system knows nothing about simply has no tooltip.

The password goes from the dialog straight to `sudo`. The caller never sees it.

![rootwhy dialog](docs/dialog-en.png)

Hovering over `-m`, for example, shows *set permission mode (as in chmod), instead of rwxr-xr-x* — the text from
`man install`. Hovering over the destination shows *⚠ an existing file — it will be replaced* when it is.

## Usage

```bash
rootwhy -r "Restart Ollama after changing its service file." systemctl restart ollama

rootwhy -r "Install the nightly backup script system-wide so the timer can run it as root." \
        -- install -m 755 -o root -g root ./build/nightly-backup.sh /usr/local/bin/nightly-backup.sh
```

| Option | |
| --- | --- |
| `-r, --reason TEXT` | why the command is needed, 10–300 characters (required) |
| `--log-dir DIR` | log directory (default `$XDG_STATE_HOME/rootwhy`, i.e. `~/.local/state/rootwhy`; also `ROOTWHY_LOG_DIR`) |
| `--preview` | show the dialog only, run nothing |

Exit code: the command's own, `130` when cancelled, `2` for usage errors. A wrong password lets `sudo` ask again
(up to 3 times), and the dialog says so.

### For AI agents — Claude Code plugin

The repository is also a Claude Code plugin: a **skill** that tells the agent when and how to use rootwhy, and a
**hook** that stops `sudo`, `pkexec`, `doas`, `run0` and `su` called directly and points to rootwhy instead. The hook
is the program itself: it runs `rootwhy hook`.

```text
/plugin marketplace add DragonKing026/rootwhy
/plugin install rootwhy@rootwhy
```

The plugin does not install the program — [`install.sh`](#installation) installs both. **Without rootwhy 3.x in `PATH`
or in `~/.local/bin` the hook warns** (a one-line message, nothing is blocked) — bare `sudo` is then not stopped. For other agents put the same rule in
`AGENTS.md`:

```markdown
Commands that need root: only through `rootwhy -r "What and why, 1–2 sentences." -- command …`, never `sudo`
directly. Tell the user a password dialog is about to appear. Exit code 130 means they cancelled — do not retry
without asking.
```

## Security model

- **Every call asks for the password** (`sudo -k`): no reuse of an earlier approval.
- **Askpass mode only under the real sudo.** `rootwhy` is its own `SUDO_ASKPASS`. In that mode it shows the dialog
  only when its parent is `sudo` started by you: process name `sudo`, real UID yours, effective UID 0 (only a
  setuid-root program has that) and, when visible, a root-owned setuid executable. An inherited environment or a fake
  `sudo` cannot turn it into a prompt that hands your password to whoever started it.
- **What you see is what runs.** The command in the dialog is read from the parent sudo's own `/proc/PID/cmdline`,
  not from anything the requester passes in: calling `sudo` directly with rootwhy as askpass and a harmless-looking
  description still shows the real command. The command name is resolved to an absolute path before sudo gets it.
  Nothing is shortened; arguments are shell-quoted, and invisible or bidirectional-control characters are shown as
  escapes such as `$'\u202e'`.
- **Clean dialog process.** The dialog runs as a separate program (`rootwhy-gtk`, found only next to `rootwhy` itself)
  with a cleared environment that keeps only display, session and language variables: `GIO_EXTRA_MODULES`, `GTK_*`,
  `QT_PLUGIN_PATH`, `LD_*` and the like cannot load foreign code into the process that holds your password.
- **Nothing runs before you agree.** Descriptions come only from `man` / its parsed page (which never executes the
  command — `--help` is never called) and warnings from `stat` of the paths involved.
- **The requester cannot put words in the tooltips.** Only the reason comes from the requester. A dishonest
  requester can lie in the reason — the command itself is what you approve.
- **The program is shown by name** (`ls`) only when `sudo` would find that very file by that name and it is safe;
  otherwise the full path and a warning.
- **Log without passwords:** date, reason, command and exit code, one line each, written before the dialog opens.

Limits: `rootwhy` cannot tell what a script will do when it runs — only what file it is and who can change it (it
warns when the program or script is not owned by root or is writable by others). Passwordless `sudo` rules
(`NOPASSWD`) or PAM methods such as a fingerprint reader skip the askpass dialog entirely.

## Installation

Two ways — the same result:

- **A. Script** — one command does all three steps below.
- **B. Manually** — the full list, step by step.

Needed in both cases: Linux (x86_64 or aarch64, glibc ≥ 2.35) with a desktop (KDE Plasma or GNOME, Wayland or X11),
`sudo`, `curl`. For the full dialog also GTK 4 ≥ 4.6 from your distribution (optional — without it the dialog uses
`zenity` or `kdialog`). No Python is needed.

### A. Script

Download it, read it, run it:

```bash
curl -fsSLO https://raw.githubusercontent.com/DragonKing026/rootwhy/main/install.sh
less install.sh
bash install.sh
```

| Step | What it does | Skip with |
| --- | --- | --- |
| 1. Programs | downloads `rootwhy` and `rootwhy-gtk` for your CPU from release `v3.0.0`, checks both against `SHA256SUMS`, puts them side by side in `~/.local/bin` | — (`--dir DIR`, `--version vX.Y.Z`) |
| 2. GTK | checks that `rootwhy-gtk` finds GTK 4; if not, prints the package to install for `dnf`, `apt`, `pacman` or `zypper` (nothing is installed as root) and says the dialog uses `zenity` / `kdialog` until then | `--no-gtk` (also skips `rootwhy-gtk`) |
| 3. Plugin | when `claude` is installed: adds the marketplace and installs `rootwhy@rootwhy` for your user (skill + hook) | `--no-plugin` |

`bash install.sh --help` lists the options. Running it again updates everything. It replaces a 2.x `rootwhy` in the
target directory.

### B. Manually

**1. The programs** — two files from the release, checked against the checksums. They must end up **in the same
directory**: `rootwhy` looks for `rootwhy-gtk` only next to itself.

```bash
ARCH=$(uname -m)    # x86_64 or aarch64
BASE=https://github.com/DragonKing026/rootwhy/releases/download/v3.0.0
cd "$(mktemp -d)"
curl -fsSLO "$BASE/rootwhy-linux-$ARCH" -O "$BASE/rootwhy-gtk-linux-$ARCH" -O "$BASE/SHA256SUMS"
sha256sum -c --ignore-missing SHA256SUMS
mkdir -p ~/.local/bin
install -m 755 "rootwhy-linux-$ARCH" ~/.local/bin/rootwhy
install -m 755 "rootwhy-gtk-linux-$ARCH" ~/.local/bin/rootwhy-gtk
rootwhy --version
```

`~/.local/bin` must be in `PATH`.

**2. GTK 4** — for the full dialog (colours, descriptions on hover): the system library, version 4.6 or newer.

```bash
sudo dnf install gtk4                  # Fedora
sudo apt install libgtk-4-1            # Debian / Ubuntu
sudo pacman -S gtk4                    # Arch
sudo zypper install libgtk-4-1         # openSUSE
ldd ~/.local/bin/rootwhy-gtk | grep 'not found' || echo OK
```

Without GTK 4 the dialog falls back to `zenity` (GNOME) or `kdialog` (KDE): the reason and the command split into
lines, without colours and hover.

**3. The Claude Code plugin** (optional, for AI agents) — skill + hook that stops bare `sudo`:

```bash
claude plugin marketplace add DragonKing026/rootwhy
claude plugin install rootwhy@rootwhy --scope user
```

or inside Claude Code: `/plugin marketplace add DragonKing026/rootwhy`, then `/plugin install rootwhy@rootwhy`. Active
in new sessions.

**4. Check** — shows the dialog, runs nothing:

```bash
rootwhy --preview -r "Testing the dialog." -- install -m 644 a /etc/b
```

### Update and uninstall

Update: run `install.sh` again, or repeat step 1 and `claude plugin marketplace update rootwhy && claude plugin update
rootwhy@rootwhy`. Uninstall: `rm ~/.local/bin/rootwhy ~/.local/bin/rootwhy-gtk`, `claude plugin uninstall
rootwhy@rootwhy` (and `~/.local/state/rootwhy` for the log).

### Building from source

```bash
sudo dnf install gtk4-devel            # Debian / Ubuntu: libgtk-4-dev
cargo build --release                  # target/release/rootwhy and target/release/rootwhy-gtk
```

Copy both files into one directory. The GTK helper is built against GTK 4.6 API only, so the binaries from the release
(built on Ubuntu 22.04) run on GTK 4.6 and newer.

### Changes since 2.x

rootwhy 3.0 is a rewrite in Rust with the same behaviour. Differences:

- no Python and no PyGObject — two native programs: `rootwhy` (logic, askpass mode, the hook, the `zenity` / `kdialog`
  fallback) and `rootwhy-gtk` (the dialog);
- GTK 3 is no longer supported (`ROOTWHY_GTK=3` is gone) — GTK 4 ≥ 4.6, or the `zenity` / `kdialog` fallback;
- command-line arguments must be valid UTF-8;
- `--help` is printed without colours;
- the plugin hook runs `rootwhy hook`; it warns (without blocking) when rootwhy 3.x is not installed.

## Language

The few fixed texts of the dialog (buttons, warnings) follow `LANG` / `LC_MESSAGES`: Polish for `pl_*`, English
otherwise; force with `ROOTWHY_LANG=en` or `ROOTWHY_LANG=pl`. Descriptions are in whatever language your man pages
are — install your distribution's translated man pages (e.g. `man-pages-pl`) to get them in your language.

## Development

```bash
cargo test                                # command model, man parsing, warnings, CLI, hook (no GTK, no sudo)
cargo build --release
target/release/rootwhy --preview -r "Testing the dialog." -- install -m 644 a /etc/b
target/release/rootwhy --screenshot out.png -r "Testing the dialog." -- systemctl restart foo   # window to PNG
```

The logo is embedded in `rootwhy-gtk` (`include_str!` of `logo.svg`). A tag `v*` builds the release files with
`.github/workflows/release.yml`.

The program that receives your password is small on purpose: `rootwhy-gtk` only draws the dialog (data in as JSON on
stdin, the password out on stdout), and the logic lives in `rootwhy`.

## Related projects

- [Salkcid/approve](https://github.com/Salkcid/approve) + [askpass](https://github.com/Salkcid/askpass) — Rust/GTK 4
  review dialog with a reason and process snapshot (Wayland only).
- [crypdick/sudoplz](https://github.com/crypdick/sudoplz) — approve agents' sudo with an SSH-key-encrypted password.

## License

[GPL-3.0-or-later](LICENSE). You may use, change and sell it, but anything you distribute that is based on it must
stay under the same license, with source code.
