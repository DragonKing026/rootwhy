#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# install.sh — install rootwhy: the program from a GitHub release (checksum verified), the GTK bindings for Python
# it needs for the full dialog, and the Claude Code plugin (skill + hook that stops bare sudo).
# https://github.com/DragonKing026/rootwhy
set -euo pipefail

REPO="DragonKing026/rootwhy"
DIR="$HOME/.local/bin"
TAG="latest"
GTK=1
PLUGIN=1
YES=0

usage() {
  cat <<'EOF'
Usage: bash install.sh [options]

Installs rootwhy in three steps:
  1. the program from a GitHub release, checked against SHA256SUMS, into ~/.local/bin
  2. the GTK bindings for Python (PyGObject + GTK 4) from your distribution, when missing (asks first; needs root)
  3. the Claude Code plugin rootwhy@rootwhy (skill + hook), when `claude` is installed

Options:
  --dir DIR       install the program into DIR (default ~/.local/bin)
  --version TAG   release tag, e.g. v2.0.0 (default: the latest release)
  --no-gtk        skip step 2 (the dialog falls back to kdialog / zenity without GTK)
  --no-plugin     skip step 3
  -y, --yes       install system packages without asking
  -h, --help      this help

Run it again to update. Uninstall: rm ~/.local/bin/rootwhy; claude plugin uninstall rootwhy@rootwhy
EOF
}

while (($#)); do
  case "$1" in
    --dir) DIR="${2:?--dir needs a directory}"; shift ;;
    --version) TAG="${2:?--version needs a tag}"; shift ;;
    --no-gtk) GTK=0 ;;
    --no-plugin) PLUGIN=0 ;;
    -y | --yes) YES=1 ;;
    -h | --help) usage; exit 0 ;;
    *) echo "install.sh: unknown option: $1 (see --help)" >&2; exit 2 ;;
  esac
  shift
done

say() { printf '\033[1m==>\033[0m %s\n' "$*"; }
ok() { printf '  \033[32m✔\033[0m %s\n' "$*"; }
warn() { printf '  \033[33m!\033[0m %s\n' "$*" >&2; }
die() { printf '  \033[31m✘\033[0m %s\n' "$*" >&2; exit 1; }

for cmd in curl sha256sum; do command -v "$cmd" >/dev/null || die "$cmd is required"; done

# ── 1. the program ──────────────────────────────────────────────────────────────────────────────────────────
say "rootwhy ($TAG) → $DIR"
if [[ "$TAG" == latest ]]; then URL="https://github.com/$REPO/releases/latest/download"
else URL="https://github.com/$REPO/releases/download/$TAG"; fi
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/rootwhy" "$URL/rootwhy" || die "download failed: $URL/rootwhy"
curl -fsSL -o "$TMP/SHA256SUMS" "$URL/SHA256SUMS" || die "download failed: $URL/SHA256SUMS"
(cd "$TMP" && grep -E '[[:space:]]\*?rootwhy$' SHA256SUMS | sha256sum -c --quiet -) || die "checksum mismatch — nothing installed"
mkdir -p "$DIR"
install -m 755 "$TMP/rootwhy" "$DIR/rootwhy"
ok "$("$DIR/rootwhy" --version) — checksum OK"
case ":$PATH:" in *":$DIR:"*) ;; *) warn "$DIR is not in PATH — add it to your shell profile" ;; esac

# ── 2. GTK for Python ───────────────────────────────────────────────────────────────────────────────────────
# rootwhy runs under /usr/bin/python3, so the bindings must be the system ones, not a venv or pip --user
has_gtk() { /usr/bin/python3 -c "import gi; gi.require_version('Gtk', '$1')" 2>/dev/null; }

if ((GTK)); then
  say "GTK bindings for Python (the full dialog: colours and descriptions on hover)"
  if [[ ! -x /usr/bin/python3 ]]; then
    die "/usr/bin/python3 not found — install Python 3 from your distribution"
  elif has_gtk 4.0; then ok "GTK 4"
  elif has_gtk 3.0; then ok "GTK 3 (GTK 4 missing — the dialog works with GTK 3 too)"
  else
    if command -v dnf >/dev/null; then PKG=(dnf install -y python3-gobject gtk4)
    elif command -v apt-get >/dev/null; then PKG=(apt-get install -y python3-gi gir1.2-gtk-4.0)
    elif command -v pacman >/dev/null; then PKG=(pacman -S --needed --noconfirm python-gobject gtk4)
    elif command -v zypper >/dev/null; then PKG=(zypper --non-interactive install python3-gobject typelib-1_0-Gtk-4_0)
    else PKG=(); fi
    if ((${#PKG[@]} == 0)); then
      warn "unknown package manager — install PyGObject and GTK 4 yourself (until then: kdialog / zenity dialog)"
    else
      echo "  Missing. To install: ${PKG[*]}"
      ANSWER=y
      if ((!YES)); then
        if [[ -t 0 ]]; then read -r -p "  Install now as root? [Y/n] " ANSWER
        else ANSWER=n; warn "not a terminal — run again with --yes, or install the packages yourself"; fi
      fi
      if [[ "${ANSWER:-y}" =~ ^[YyTt]?$ ]]; then
        # in a terminal plain sudo; without one (e.g. an agent) rootwhy itself asks for the password in a dialog
        if [[ -t 0 ]]; then sudo -- "${PKG[@]}"
        else "$DIR/rootwhy" -r "Install the GTK bindings for Python, needed for the full rootwhy password dialog." -- "${PKG[@]}"; fi
        if has_gtk 4.0 || has_gtk 3.0; then ok "GTK installed"; else warn "still not importable — check the output above"; fi
      else
        warn "skipped — the dialog falls back to kdialog / zenity"
      fi
    fi
  fi
fi

# ── 3. Claude Code plugin ───────────────────────────────────────────────────────────────────────────────────
if ((PLUGIN)); then
  say "Claude Code plugin rootwhy@rootwhy (skill + hook that stops bare sudo)"
  if ! command -v claude >/dev/null; then
    ok "claude not found — skipped (later: claude plugin marketplace add $REPO && claude plugin install rootwhy@rootwhy)"
  else
    if claude plugin marketplace list 2>/dev/null | grep -qw rootwhy; then
      claude plugin marketplace update rootwhy >/dev/null 2>&1 && ok "marketplace rootwhy updated" || warn "marketplace update failed"
    else
      claude plugin marketplace add "$REPO" >/dev/null 2>&1 && ok "marketplace rootwhy added" || die "claude plugin marketplace add $REPO failed"
    fi
    if claude plugin list 2>/dev/null | grep -q 'rootwhy@rootwhy'; then
      claude plugin update rootwhy@rootwhy >/dev/null 2>&1 || true
      ok "plugin already installed (updated where a newer version exists)"
    else
      claude plugin install rootwhy@rootwhy --scope user >/dev/null 2>&1 && ok "plugin installed — active in new Claude Code sessions" \
        || die "claude plugin install rootwhy@rootwhy failed"
    fi
  fi
fi

say "Done. Try: rootwhy --preview -r \"Testing the dialog.\" -- true"
