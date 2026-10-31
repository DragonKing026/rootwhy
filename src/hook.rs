// SPDX-License-Identifier: GPL-3.0-or-later
//! PreToolUse hook: stop sudo / pkexec / doas / run0 / su called directly in a Bash command; point to rootwhy.
//! Words are split like the shell does (quotes and backslashes removed), heredoc bodies end where bash ends them,
//! `bash -c` / `sh -c` / `eval` strings are checked too, and launchers such as `xargs sudo` count. When the command
//! cannot be parsed and mentions one of the names, it is blocked. A guardrail for the agent — the real protection is
//! the password, which only the user types.

use std::collections::BTreeSet;
use std::io::Read;

const BLOCKED: [&str; 5] = ["sudo", "pkexec", "doas", "run0", "su"];
const PREFIXES: [&str; 16] = [
    "exec", "time", "nohup", "command", "builtin", "env", "nice", "!", "{", "if", "then", "else", "elif", "while", "until", "do",
];
const LAUNCHERS: [&str; 14] = [
    "xargs",
    "timeout",
    "stdbuf",
    "ionice",
    "setsid",
    "chrt",
    "taskset",
    "flock",
    "watch",
    "unbuffer",
    "systemd-run",
    "script",
    "find",
    "parallel",
];
const SHELLS: [&str; 5] = ["bash", "sh", "zsh", "dash", "ksh"];
const PUNCT: &str = ";&|()<>";

/// `\w+=.*` (fullmatch): an assignment such as FOO=1 in front of a command.
fn is_assignment(w: &str) -> bool {
    let Some(eq) = w.find('=') else { return false };
    let name = &w[..eq];
    !name.is_empty() && name.chars().all(|c| c.is_alphanumeric() || c == '_') && !w[eq + 1..].contains('\n')
}

fn is_operator(tok: &str) -> bool {
    !tok.is_empty() && tok.chars().all(|c| PUNCT.contains(c))
}

fn basename(w: &str) -> &str {
    w.rsplit('/').next().unwrap_or("")
}

/// `(?<!<)<<(-?)\s*(['"]?)([^\s'"<>|&;()]+)\2` — all heredoc starts on a line: (dash, delimiter).
fn heredocs_in(line: &str) -> Vec<(bool, String)> {
    let c: Vec<char> = line.chars().collect();
    let is_ws = crate::util::py_isspace;
    let delim_char = |ch: char| !is_ws(ch) && !"'\"<>|&;()".contains(ch);
    let try_at = |start: usize, dash: bool| -> Option<(String, usize)> {
        let mut p = start + 2 + usize::from(dash);
        while p < c.len() && is_ws(c[p]) {
            p += 1;
        }
        let quote = if p < c.len() && (c[p] == '\'' || c[p] == '"') {
            p += 1;
            Some(c[p - 1])
        } else {
            None
        };
        let s = p;
        while p < c.len() && delim_char(c[p]) {
            p += 1;
        }
        if p == s {
            return None;
        }
        let delim: String = c[s..p].iter().collect();
        if let Some(q) = quote {
            if p >= c.len() || c[p] != q {
                return None;
            }
            p += 1;
        }
        Some((delim, p))
    };
    let mut out = Vec::new();
    let mut i = 0;
    while i < c.len() {
        if c[i] == '<' && i + 1 < c.len() && c[i + 1] == '<' && (i == 0 || c[i - 1] != '<') {
            let hit = if i + 2 < c.len() && c[i + 2] == '-' { try_at(i, true).map(|r| (true, r)) } else { None }
                .or_else(|| try_at(i, false).map(|r| (false, r)));
            if let Some((dash, (delim, end))) = hit {
                out.push((dash, delim));
                i = end;
                continue;
            }
        }
        i += 1;
    }
    out
}

/// Drop heredoc bodies: after a line with <<DELIM, lines up to one exactly equal to DELIM (tabs stripped for <<-).
pub fn strip_heredocs(cmd: &str) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut pending: std::collections::VecDeque<(bool, String)> = Default::default();
    for line in cmd.split('\n') {
        if let Some((dash, delim)) = pending.front() {
            let cmp = if *dash { line.trim_start_matches('\t') } else { line };
            if cmp == delim {
                pending.pop_front();
            }
            continue;
        }
        out.push(line);
        pending.extend(heredocs_in(line));
    }
    out.join("\n")
}

/// The text cannot be split into words (unbalanced quote, trailing backslash).
#[derive(Debug, PartialEq, Eq)]
pub struct ParseError;

/// Python's shlex.shlex(text, posix=True, punctuation_chars=";&|()<>") with whitespace_split and no comments.
/// Err = unbalanced quote or a trailing backslash.
pub fn shlex_split(text: &str) -> Result<Vec<String>, ParseError> {
    let chars: Vec<char> = text.chars().collect();
    let mut pos = 0;
    let mut tokens = Vec::new();
    let is_ws = |c: char| matches!(c, ' ' | '\t' | '\r' | '\n');
    loop {
        let mut token = String::new();
        let mut quoted = false;
        let mut state = ' ';
        let mut escaped = ' ';
        loop {
            let next = chars.get(pos).copied();
            if next.is_some() {
                pos += 1;
            }
            match state {
                ' ' => match next {
                    None => break,
                    Some(c) if is_ws(c) => {
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                    Some('\\') => {
                        escaped = 'a';
                        state = '\\';
                    }
                    Some(c) if PUNCT.contains(c) => {
                        token.push(c);
                        state = 'c';
                    }
                    Some(c @ ('\'' | '"')) => state = c,
                    Some(c) => {
                        token.push(c);
                        state = 'a';
                    }
                },
                '\'' | '"' => {
                    quoted = true;
                    match next {
                        None => return Err(ParseError),
                        Some(c) if c == state => state = 'a',
                        Some('\\') if state == '"' => {
                            escaped = state;
                            state = '\\';
                        }
                        Some(c) => token.push(c),
                    }
                }
                '\\' => match next {
                    None => return Err(ParseError),
                    Some(c) => {
                        // only the quote itself or the backslash may be escaped within quotes
                        if (escaped == '"' || escaped == '\'') && c != state && c != escaped {
                            token.push(state);
                        }
                        token.push(c);
                        state = escaped;
                    }
                },
                _ => match next {
                    // 'a' (word) and 'c' (punctuation run)
                    None => break,
                    Some(c) if is_ws(c) => {
                        state = ' ';
                        if !token.is_empty() || quoted {
                            break;
                        }
                    }
                    Some(c) if state == 'c' => {
                        if PUNCT.contains(c) {
                            token.push(c);
                        } else {
                            pos -= 1; // push back
                            break;
                        }
                    }
                    Some(c @ ('\'' | '"')) => state = c,
                    Some('\\') => {
                        escaped = 'a';
                        state = '\\';
                    }
                    Some(c) if !PUNCT.contains(c) => token.push(c),
                    Some(_) => {
                        pos -= 1;
                        break;
                    }
                },
            }
        }
        if !quoted && token.is_empty() {
            return Ok(tokens); // end of input
        }
        tokens.push(token);
    }
}

fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-'
}

/// `(?<![\w.-])NAME(?![\w.-])` anywhere in the text.
fn mentions(cmd: &str, name: &str) -> bool {
    let mut from = 0;
    while let Some(off) = cmd[from..].find(name) {
        let start = from + off;
        let end = start + name.len();
        let before = cmd[..start].chars().next_back();
        let after = cmd[end..].chars().next();
        if !before.is_some_and(is_word_char) && !after.is_some_and(is_word_char) {
            return true;
        }
        from = start + cmd[start..].chars().next().map_or(1, char::len_utf8);
    }
    false
}

pub fn blocked_words(cmd: &str, depth: usize) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    if depth > 3 {
        return found;
    }
    let text = strip_heredocs(cmd).replace(['\n', '`'], " ; ").replace("$(", " ; ( ");
    let Ok(tokens) = shlex_split(&text) else {
        // unbalanced quotes: fail closed when a name appears at all
        return BLOCKED.iter().filter(|w| mentions(cmd, w)).map(|w| w.to_string()).collect();
    };
    let mut segment: Vec<String> = Vec::new();
    for tok in tokens.into_iter().chain(std::iter::once(";".to_string())) {
        if !is_operator(&tok) {
            segment.push(tok);
            continue;
        }
        let mut words = std::mem::take(&mut segment);
        while words.first().is_some_and(|w| PREFIXES.contains(&w.as_str()) || is_assignment(w)) {
            words.remove(0);
        }
        if words.is_empty() {
            continue;
        }
        let name = basename(&words[0]).to_string();
        if BLOCKED.contains(&name.as_str()) {
            found.insert(name);
        } else if LAUNCHERS.contains(&name.as_str()) {
            for w in &words[1..] {
                let b = basename(w);
                if BLOCKED.contains(&b) {
                    found.insert(b.to_string());
                }
            }
        } else if SHELLS.contains(&name.as_str()) && words.len() >= 3 && words[1..words.len() - 1].iter().any(|w| w == "-c") {
            let idx = words.iter().position(|w| w == "-c").unwrap();
            found.extend(blocked_words(&words[idx + 1], depth + 1));
        } else if name == "eval" {
            found.extend(blocked_words(&words[1..].join(" "), depth + 1));
        }
    }
    found
}

pub fn message(found: &[String]) -> String {
    format!(
        "Blocked: {} called directly. Use rootwhy instead — the user sees a dialog with your reason \
         and the command, and types the password there:\n\
         \x20 rootwhy -r \"What and why, 1–2 sentences in the user's language.\" -- command args…\n\
         Tell the user first that a password dialog is about to appear. Exit code 130 = cancelled: do not retry \
         without asking. (If this project defines its own wrapper around rootwhy, use that.)",
        found.join(", ")
    )
}

/// Decide for one JSON document: Some(message) = block (exit 2).
pub fn decide(input: &[u8]) -> Option<String> {
    let data: serde_json::Value = serde_json::from_slice(input).ok()?;
    let cmd = data.get("tool_input").and_then(|t| t.get("command")).and_then(|c| c.as_str()).unwrap_or("");
    let found: Vec<String> = blocked_words(cmd, 0).into_iter().collect();
    if found.is_empty() {
        None
    } else {
        Some(message(&found))
    }
}

/// `rootwhy hook`: PreToolUse JSON on stdin; exit 2 with the message on stderr when sudo & co. is called directly.
pub fn run() -> i32 {
    let mut input = Vec::new();
    if std::io::stdin().read_to_end(&mut input).is_err() {
        return 0;
    }
    match decide(&input) {
        None => 0,
        Some(m) => {
            eprintln!("{m}");
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_cmd(command: &str) -> i32 {
        let data = serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}}).to_string();
        if decide(data.as_bytes()).is_some() {
            2
        } else {
            0
        }
    }

    #[test]
    fn blocks_command_words() {
        for cmd in [
            "sudo dnf install htop",
            "cd /tmp && sudo rm x",
            "ls | sudo tee /etc/x",
            "FOO=1 sudo true",
            "/usr/bin/sudo -k true",
            "pkexec true",
            "echo $(sudo cat /etc/shadow)",
            "nohup sudo true &",
            "doas true",
            "su -c id",
            "run0 true",
            "x=1\nsudo true",
            "'sudo' true",
            "s\\udo true",
            "\"/usr/bin/sudo\" true",
            "find . | xargs sudo rm",
            "timeout 5 sudo true",
            "bash -c \"sudo true\"",
            "sh -c 'cd / && sudo ls'",
            "eval \"sudo true\"",
            "`sudo true`",
            "{ sudo true; }",
            "if true; then sudo true; fi",
            "cat <<EOF\nx\nEOF\nsudo true",    // body ended, next line runs
            "cat <<EOF\n EOF\nEOF\nsudo true", // ' EOF' does not end it, 'EOF' does
            "echo 'unbalanced; sudo true",     // cannot parse -> fail closed
        ] {
            assert_eq!(run_cmd(cmd), 2, "{cmd}");
        }
    }

    #[test]
    fn allows_mentions() {
        for cmd in [
            "git commit -m \"never call sudo directly\"",
            "grep -rn sudo README.md",
            "rootwhy -r 'Why x.' -- dnf up",
            "cat <<EOF > notes.txt\nsudo is blocked\nEOF\necho ok",
            "man sudo",
            "echo 'a; sudo b'",
            "jako-root -o 'Opis zmiany.' true",
            "pseudo-cmd",
            "visudo -c",
            "cat <<'EOF'\nsudo true\nEOF",
            "cat <<-EOF\n\tsudo x\n\tEOF\necho ok",
            "xargs echo sudo-free",
            "bash -c 'echo hi'",
            "echo \"$(date)\" # sudo later",
        ] {
            assert_eq!(run_cmd(cmd), 0, "{cmd}");
        }
    }

    #[test]
    fn bad_input_passes() {
        assert!(decide(b"not json").is_none());
        assert!(decide(&[0xff, 0xfe]).is_none());
        assert!(decide(b"{}").is_none());
    }

    #[test]
    fn message_lists_sorted_names() {
        let m = decide(br#"{"tool_input":{"command":"sudo a; pkexec b"}}"#).unwrap();
        assert!(m.starts_with("Blocked: pkexec, sudo called directly. Use rootwhy instead"));
    }

    #[test]
    fn shlex_matches_python() {
        let s = |t: &str| shlex_split(t).unwrap();
        assert_eq!(s("a 'b c' \"d\\\"e\" f\\ g"), ["a", "b c", "d\"e", "f g"]);
        assert_eq!(s("a;b&&c||d"), ["a", ";", "b", "&&", "c", "||", "d"]);
        assert_eq!(s("a >out 2>&1"), ["a", ">", "out", "2", ">&", "1"]);
        assert_eq!(s("'' x"), ["", "x"]);
        assert_eq!(s("\"a\\b\""), ["a\\b"]);
        assert!(shlex_split("a 'b").is_err());
        assert!(shlex_split("a \\").is_err());
    }

    #[test]
    fn heredoc_detection() {
        assert_eq!(heredocs_in("cat <<EOF"), vec![(false, "EOF".to_string())]);
        assert_eq!(heredocs_in("cat <<-'EOF' x"), vec![(true, "EOF".to_string())]);
        assert_eq!(heredocs_in("cat <<< word"), Vec::<(bool, String)>::new());
        assert_eq!(heredocs_in("a <<A <<\"B\""), vec![(false, "A".to_string()), (false, "B".to_string())]);
    }
}
