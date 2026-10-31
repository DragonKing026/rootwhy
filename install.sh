#!/usr/bin/env bash
# SPDX-License-Identifier: GPL-3.0-or-later
# install.sh — install rootwhy: the two programs (rootwhy and the GTK 4 dialog helper rootwhy-gtk) from a GitHub
# release (checksums verified) and the Claude Code plugin (skill + hook that stops bare sudo).
# https://github.com/DragonKing026/rootwhy
set -euo pipefail

REPO="DragonKing026/rootwhy"
DIR="$HOME/.local/bin"
TAG="v3.0.0"
GTK=1
PLUGIN=1

usage() {
  cat <<'EOF'
Usage: bash install.sh [options]

Installs rootwhy in three steps:
  1. the programs rootwhy and rootwhy-gtk (for your CPU: x86_64 or aarch64) from a GitHub release, checked against
     SHA256SUMS, into ~/.local/bin
  2. a check for GTK 4 (libgtk-4.so.1), which rootwhy-gtk needs; if it is missing, you are told what to install
     (until then the password dialog falls back to zenity / kdialog)
  3. the Claude Code plugin rootwhy@rootwhy (skill + hook), when `claude` is installed

Options:
  --dir DIR       install the programs into DIR (default ~/.local/bin; both go to the same directory)
  --version TAG   release tag (default: v3.0.0)
  --no-gtk        do not install rootwhy-gtk and skip step 2 (the dialog uses zenity / kdialog)
  --no-plugin     skip step 3
  -y, --yes       accepted for compatibility; nothing is installed as root any more
  -h, --help      this help

Run it again to update. Uninstall: rm ~/.local/bin/rootwhy ~/.local/bin/rootwhy-gtk; claude plugin uninstall rootwhy@rootwhy
EOF
}

while (($#)); do
  case "$1" in
    --dir) DIR="${2:?--dir needs a directory}"; shift ;;
    --version) TAG="${2:?--version needs a tag}"; shift ;;
    --no-gtk) GTK=0 ;;
    --no-plugin) PLUGIN=0 ;;
    -y | --yes) ;;
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

# ── 1. the programs ─────────────────────────────────────────────────────────────────────────────────────────
case "$(uname -m)" in
  x86_64 | amd64) ARCH=x86_64 ;;
  aarch64 | arm64) ARCH=aarch64 ;;
  *) die "unsupported CPU: $(uname -m) (releases exist for x86_64 and aarch64)" ;;
esac
NAMES=(rootwhy)
if ((GTK)); then NAMES+=(rootwhy-gtk); fi
say "rootwhy ($TAG, $ARCH) → $DIR"
URL="https://github.com/$REPO/releases/download/$TAG"
TMP=$(mktemp -d)
trap 'rm -rf "$TMP"' EXIT
curl -fsSL -o "$TMP/SHA256SUMS" "$URL/SHA256SUMS" || die "download failed: $URL/SHA256SUMS"
for n in "${NAMES[@]}"; do
  curl -fsSL -o "$TMP/$n-linux-$ARCH" "$URL/$n-linux-$ARCH" || die "download failed: $URL/$n-linux-$ARCH"
  (cd "$TMP" && grep -E "[[:space:]]\*?$n-linux-$ARCH\$" SHA256SUMS | sha256sum -c --quiet -) || die "checksum mismatch ($n) — nothing installed"
done
mkdir -p "$DIR"
for n in "${NAMES[@]}"; do install -m 755 "$TMP/$n-linux-$ARCH" "$DIR/$n"; done
ok "$("$DIR/rootwhy" --version) — checksums OK"
case ":$PATH:" in *":$DIR:"*) ;; *) warn "$DIR is not in PATH — add it to your shell profile" ;; esac

# ── 2. GTK 4 ────────────────────────────────────────────────────────────────────────────────────────────────
# rootwhy-gtk is linked against the system GTK 4 (>= 4.6); without it the dialog falls back to zenity / kdialog
if ((GTK)); then
  say "GTK 4 for the full dialog (colours and descriptions on hover)"
  if ldd "$DIR/rootwhy-gtk" 2>/dev/null | grep -q 'not found'; then
    warn "GTK 4 (libgtk-4.so.1) or one of its libraries is missing — until you install it, the dialog uses zenity / kdialog"
    echo "  Install GTK 4 for your distribution:"
    echo "    Fedora:           sudo dnf install gtk4"
    echo "    Debian / Ubuntu:  sudo apt install libgtk-4-1"
    echo "    Arch:             sudo pacman -S gtk4"
    echo "    openSUSE:         sudo zypper install libgtk-4-1"
  else
    ok "GTK 4 found"
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
