# SPDX-License-Identifier: GPL-3.0-or-later
"""PreToolUse hook: stop sudo / pkexec / doas / run0 / su called directly in a Bash command; point to rootwhy.
Words are split like the shell does (quotes and backslashes removed), heredoc bodies end where bash ends them,
`bash -c` / `sh -c` / `eval` strings are checked too, and launchers such as `xargs sudo` count. When the command
cannot be parsed and mentions one of the names, it is blocked. A guardrail for the agent — the real protection is
the password, which only the user types."""
import json
import re
import shlex
import sys

BLOCKED = {"sudo", "pkexec", "doas", "run0", "su"}
PREFIXES = {"exec", "time", "nohup", "command", "builtin", "env", "nice", "!", "{", "if", "then", "else", "elif",
            "while", "until", "do"}
LAUNCHERS = {"xargs", "timeout", "stdbuf", "ionice", "setsid", "chrt", "taskset", "flock", "watch", "unbuffer",
             "systemd-run", "script", "find", "parallel"}
SHELLS = {"bash", "sh", "zsh", "dash", "ksh"}
OPERATORS = re.compile(r"[;&|()<>]+")
HEREDOC = re.compile(r"(?<!<)<<(-?)\s*(['\"]?)([^\s'\"<>|&;()]+)\2")


def strip_heredocs(cmd: str) -> str:
    """Drop heredoc bodies: after a line with <<DELIM, lines up to one exactly equal to DELIM (tabs stripped
    for <<-)."""
    out, pending = [], []
    for line in cmd.split("\n"):
        if pending:
            dash, delim = pending[0]
            if (line.lstrip("\t") if dash else line) == delim:
                pending.pop(0)
            continue
        out.append(line)
        pending += [(m.group(1) == "-", m.group(3)) for m in HEREDOC.finditer(line)]
    return "\n".join(out)


def blocked_words(cmd: str, depth: int = 0) -> set[str]:
    if depth > 3:
        return set()
    text = strip_heredocs(cmd).replace("\n", " ; ").replace("`", " ; ").replace("$(", " ; ( ")
    try:
        lexer = shlex.shlex(text, posix=True, punctuation_chars=";&|()<>")
        lexer.whitespace_split = True
        lexer.commenters = ""
        tokens = list(lexer)
    except ValueError:  # unbalanced quotes: fail closed when a name appears at all
        return {w for w in BLOCKED if re.search(rf"(?<![\w.-]){w}(?![\w.-])", cmd)}
    found: set[str] = set()
    segment: list[str] = []
    for tok in [*tokens, ";"]:
        if not OPERATORS.fullmatch(tok):
            segment.append(tok)
            continue
        words = list(segment)
        segment = []
        while words and (words[0] in PREFIXES or re.fullmatch(r"\w+=.*", words[0])):
            words.pop(0)
        if not words:
            continue
        name = words[0].rsplit("/", 1)[-1]
        if name in BLOCKED:
            found.add(name)
        elif name in LAUNCHERS:
            found |= {w.rsplit("/", 1)[-1] for w in words[1:]} & BLOCKED
        elif name in SHELLS and "-c" in words[1:-1]:
            found |= blocked_words(words[words.index("-c") + 1], depth + 1)
        elif name == "eval":
            found |= blocked_words(" ".join(words[1:]), depth + 1)
    return found


def main() -> int:
    try:
        data = json.load(sys.stdin)
    except (json.JSONDecodeError, ValueError):
        return 0
    cmd = str((data.get("tool_input") or {}).get("command") or "")
    found = sorted(blocked_words(cmd))
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
