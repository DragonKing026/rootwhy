# SPDX-License-Identifier: GPL-3.0-or-later
"""PreToolUse hook: stop sudo / pkexec / doas / run0 / su called directly in a Bash command; point to rootwhy.
Only a command word counts — the same name inside quotes, a heredoc body or as an argument does not."""
import json
import re
import sys

BLOCKED = {"sudo", "pkexec", "doas", "run0", "su"}
PREFIXES = {"exec", "time", "nohup", "command", "builtin", "env", "nice", "!"}


def command_words(cmd: str) -> list[str]:
    # heredoc bodies are data
    cmd = re.sub(r"<<-?\s*(['\"]?)(\w+)\1[^\n]*\n.*?\n\s*\2\s*(?:\n|$)", "\n", cmd, flags=re.DOTALL)
    # quoted strings are arguments (keep $(…) / `…` inside double quotes out of scope: rare for root commands)
    cmd = re.sub(r"'[^']*'|\"(?:\\.|[^\"\\])*\"", "''", cmd)
    words = []
    for segment in re.split(r"\|\|?|&&|;|&|\n|\$\(|`|\(|\)|\{|\}", cmd):
        tokens = segment.split()
        while tokens and (tokens[0] in PREFIXES or re.fullmatch(r"\w+=\S*", tokens[0])):
            tokens.pop(0)
        if tokens:
            words.append(tokens[0].rsplit("/", 1)[-1])
    return words


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        return 0
    cmd = str((data.get("tool_input") or {}).get("command") or "")
    found = sorted({w for w in command_words(cmd) if w in BLOCKED})
    if not found:
        return 0
    print(f"Blocked: {', '.join(found)} called directly. Use rootwhy instead — the user sees a dialog with your reason "
          "and the command, and types the password there:\n"
          '  rootwhy -r "What and why, 1–2 sentences in the user\'s language." -- command args…\n'
          "Tell the user first that a password dialog is about to appear. Exit code 130 = cancelled: do not retry "
          "without asking. (If this project defines its own wrapper around rootwhy, use that.)", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main())
