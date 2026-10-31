// SPDX-License-Identifier: GPL-3.0-or-later
//! Command model: argv split into display tokens (no descriptions: those come from the system's man pages),
//! roles of the arguments rootwhy checks, and the line layout for the dialog.

use crate::util::{basename, realpath, unsafe_for_root, which, SECURE_PATH};
use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

pub const ONE_LINE_MAX: usize = 70; // commands up to this many characters stay on one line

/// Commands whose first argument is a subcommand (shown on the first line).
const SUBCOMMAND_TOOLS: &[&str] = &[
    "systemctl",
    "dnf",
    "dnf5",
    "apt",
    "apt-get",
    "flatpak",
    "snap",
    "tailscale",
    "docker",
    "podman",
    "loginctl",
    "hostnamectl",
    "timedatectl",
    "firewall-cmd",
    "ufw",
];

/// Options that take the next argument as their value, when man cannot tell (it is also asked).
fn builtin_value_options() -> &'static HashMap<&'static str, HashSet<&'static str>> {
    static M: OnceLock<HashMap<&'static str, HashSet<&'static str>>> = OnceLock::new();
    M.get_or_init(|| {
        let mut m: HashMap<&str, HashSet<&str>> = HashMap::new();
        let set = |v: &[&'static str]| v.iter().copied().collect::<HashSet<_>>();
        m.insert("bash", set(&["-c"]));
        m.insert("sh", set(&["-c"]));
        for c in ["cp", "mv", "ln"] {
            m.insert(c, set(&["-t", "--target-directory"]));
        }
        m.insert("install", set(&["-m", "--mode", "-o", "--owner", "-g", "--group", "-t", "--target-directory"]));
        m.insert("mkdir", set(&["-m", "--mode"]));
        m.insert("dnf", set(&["--repo", "--setopt"]));
        m.insert("dnf5", set(&["--repo", "--setopt"]));
        m.insert("journalctl", set(&["-n", "--lines", "-u", "--unit"]));
        m.insert("mount", set(&["-o", "--options", "-t", "--types"]));
        m.insert("sed", set(&["-e", "--expression"]));
        m.insert("sysctl", set(&["-p", "--load"]));
        m.insert("usermod", set(&["-G", "--groups", "-d", "--home", "-s", "--shell"]));
        m
    })
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Command,
    Subcommand,
    Option,
    Value,
    Path,
    Arg,
    Marker,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Command => "command",
            Kind::Subcommand => "subcommand",
            Kind::Option => "option",
            Kind::Value => "value",
            Kind::Path => "path",
            Kind::Arg => "arg",
            Kind::Marker => "marker",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    None,
    Destination,
    Script,
    Mode,
    Owner,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub shown: String, // what the dialog shows (quoted, control characters made visible)
    pub kind: Kind,
    pub arg: String,    // the original argument (for a joined value: just the value)
    pub index: usize,   // position in argv
    pub option: String, // for options and their values: the option name
    pub role: Role,
    pub joined: bool,       // no space before this token (value of --opt=value)
    pub notes: Vec<String>, // tooltip lines — filled by explain()
}

impl Token {
    pub fn new(shown: &str, kind: Kind, arg: &str, index: usize) -> Token {
        Token {
            shown: shown.to_string(),
            kind,
            arg: arg.to_string(),
            index,
            option: String::new(),
            role: Role::None,
            joined: false,
            notes: Vec::new(),
        }
    }
}

/// Zero-width and bidirectional-control characters: printable for Python, invisible (or misleading) on screen.
fn is_invisible(c: char) -> bool {
    matches!(c as u32, 0x200B | 0x200C | 0x200D | 0x2060 | 0xFEFF | 0x202A..=0x202E | 0x2066..=0x2069)
}

/// Approximation of Python's `str.isprintable` for one character (no table of unassigned code points).
fn is_printable(c: char) -> bool {
    if c == ' ' {
        return true;
    }
    let u = c as u32;
    if c.is_control() || c.is_whitespace() {
        return false;
    }
    // Cf (format), Co (private use), noncharacters
    let cf = matches!(u, 0xAD | 0x600..=0x605 | 0x61C | 0x6DD | 0x70F | 0x890 | 0x891 | 0x8E2 | 0x180E | 0x200B..=0x200F
        | 0x202A..=0x202E | 0x2060..=0x2064 | 0x2066..=0x206F | 0xFEFF | 0xFFF9..=0xFFFB | 0x110BD | 0x110CD
        | 0x13430..=0x1343F | 0x1BCA0..=0x1BCA3 | 0x1D173..=0x1D17A | 0xE0001 | 0xE0020..=0xE007F);
    let co = matches!(u, 0xE000..=0xF8FF | 0xF0000..=0xFFFFD | 0x100000..=0x10FFFD);
    let nonchar = matches!(u, 0xFDD0..=0xFDEF) || (u & 0xFFFE) == 0xFFFE;
    !(cf || co || nonchar)
}

fn is_plain_word(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./~-".contains(c))
}

/// Quote like a shell would need, and make invisible/control characters visible ($'…' form).
pub fn shell_quote(s: &str) -> String {
    if is_plain_word(s) {
        return s.to_string();
    }
    if s.chars().any(|c| !is_printable(c) || is_invisible(c)) {
        let mut out = String::from("$'");
        for ch in s.chars() {
            match ch {
                '\\' | '\'' => {
                    out.push('\\');
                    out.push(ch);
                }
                '\n' => out.push_str("\\n"),
                '\t' => out.push_str("\\t"),
                c if !is_printable(c) || is_invisible(c) => {
                    if (c as u32) < 0x100 {
                        out.push_str(&format!("\\x{:02x}", c as u32));
                    } else {
                        out.push_str(&format!("\\u{:04x}", c as u32));
                    }
                }
                c => out.push(c),
            }
        }
        out.push('\'');
        return out;
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn looks_like_path(s: &str) -> bool {
    s.starts_with(['/', '~'])
        || s.starts_with("./")
        || s.starts_with("../")
        || (s.contains('/') && !s.starts_with('-') && !s.contains("://"))
}

/// The bare name when sudo would find this very file by it (the full path is in the tooltip); otherwise the path —
/// also when the file is unsafe, so the warning stays next to what is shown.
fn short_command(path: &str) -> String {
    let name = basename(path);
    if let Some(found) = which(name, SECURE_PATH) {
        if path.starts_with('/') && realpath(&found) == realpath(path) && !unsafe_for_root(path) && !unsafe_for_root(&found) {
            return name.to_string();
        }
    }
    path.to_string()
}

fn is_negative_int(a: &str) -> bool {
    a.len() > 1 && a.starts_with('-') && a[1..].chars().all(|c| c.is_ascii_digit())
}

fn slice_from(s: &str, n: usize) -> String {
    s.chars().skip(n).collect()
}

/// Split argv into display tokens. `value_options`: options known to take a value for this command (from man).
pub fn tokenize(argv: &[String], value_options: Option<&HashSet<String>>) -> Vec<Token> {
    if argv.is_empty() {
        return Vec::new();
    }
    let cmd = basename(&argv[0]).to_string();
    let mut takes: HashSet<String> =
        builtin_value_options().get(cmd.as_str()).map(|s| s.iter().map(|x| x.to_string()).collect()).unwrap_or_default();
    if let Some(extra) = value_options {
        takes.extend(extra.iter().cloned());
    }
    let mut tokens = vec![Token::new(&shell_quote(&short_command(&argv[0])), Kind::Command, &argv[0], 0)];
    let mut sub_done = !SUBCOMMAND_TOOLS.contains(&cmd.as_str());
    let mut end_of_options = false;
    let mut i = 1;
    while i < argv.len() {
        let a = &argv[i];
        let alen = a.chars().count();
        if !end_of_options && a == "--" {
            tokens.push(Token::new("--", Kind::Marker, a, i));
            end_of_options = true;
        } else if !end_of_options && a.starts_with("--") && alen > 2 {
            if let Some((name, value)) = a.split_once('=') {
                let mut o = Token::new(&shell_quote(&format!("{name}=")), Kind::Option, a, i);
                o.option = name.to_string();
                tokens.push(o);
                let mut v = Token::new(&shell_quote(value), Kind::Value, value, i);
                v.option = name.to_string();
                v.joined = true;
                tokens.push(v);
            } else {
                let mut o = Token::new(&shell_quote(a), Kind::Option, a, i);
                o.option = a.clone();
                tokens.push(o);
                if takes.contains(a.as_str()) && i + 1 < argv.len() {
                    i += 1;
                    let mut v = Token::new(&shell_quote(&argv[i]), Kind::Value, &argv[i], i);
                    v.option = a.clone();
                    tokens.push(v);
                }
            }
        } else if !end_of_options && a.starts_with('-') && alen > 1 && !is_negative_int(a) {
            let short: String = a.chars().take(2).collect();
            if takes.contains(&short) && alen > 2 {
                let mut o = Token::new(&shell_quote(&short), Kind::Option, a, i);
                o.option = short.clone();
                tokens.push(o);
                let rest = slice_from(a, 2);
                let mut v = Token::new(&shell_quote(&rest), Kind::Value, &rest, i);
                v.option = short;
                v.joined = true;
                tokens.push(v);
            } else {
                let mut o = Token::new(&shell_quote(a), Kind::Option, a, i);
                o.option = a.clone();
                tokens.push(o);
                let last = format!("-{}", a.chars().last().unwrap_or('-'));
                if (takes.contains(a.as_str()) || (alen > 2 && takes.contains(&last))) && i + 1 < argv.len() {
                    i += 1;
                    let mut v = Token::new(&shell_quote(&argv[i]), Kind::Value, &argv[i], i);
                    v.option = if takes.contains(a.as_str()) { a.clone() } else { last };
                    tokens.push(v);
                }
            }
        } else if !sub_done {
            tokens.push(Token::new(&shell_quote(a), Kind::Subcommand, a, i));
            sub_done = true;
        } else {
            let kind = if looks_like_path(a) { Kind::Path } else { Kind::Arg };
            tokens.push(Token::new(&shell_quote(a), kind, a, i));
        }
        i += 1;
    }
    assign_roles(&cmd, &mut tokens);
    tokens
}

/// Mark the arguments rootwhy checks for a warning: a destination that gets replaced, a script root will run,
/// a permission mode, a new owner.
fn assign_roles(cmd: &str, tokens: &mut [Token]) {
    let positional: Vec<usize> =
        tokens.iter().enumerate().filter(|(_, t)| matches!(t.kind, Kind::Path | Kind::Arg)).map(|(n, _)| n).collect();
    let has_opt = |names: &[&str]| tokens.iter().any(|t| t.kind == Kind::Option && names.contains(&t.option.as_str()));
    let (dest, script) = (
        matches!(cmd, "cp" | "mv" | "install")
            && positional.len() > 1
            && !has_opt(&["-t", "--target-directory", "-d", "--directory"]),
        matches!(cmd, "bash" | "sh") && !positional.is_empty() && !has_opt(&["-c"]),
    );
    if dest {
        tokens[*positional.last().unwrap()].role = Role::Destination;
    } else if script {
        tokens[positional[0]].role = Role::Script;
    } else if cmd == "chmod" && !positional.is_empty() {
        tokens[positional[0]].role = Role::Mode;
    } else if matches!(cmd, "chown" | "chgrp") && !positional.is_empty() {
        tokens[positional[0]].role = Role::Owner;
    }
}

/// Lines for the dialog: short commands on one line; otherwise command (+ subcommand) first, then each option
/// with its value, each argument on its own line.
pub fn layout(tokens: &[Token]) -> Vec<Vec<Token>> {
    if tokens.is_empty() {
        return Vec::new();
    }
    let total: usize = tokens.iter().map(|t| t.shown.chars().count() + usize::from(!t.joined)).sum();
    if total <= ONE_LINE_MAX {
        return vec![tokens.to_vec()];
    }
    let mut lines: Vec<Vec<Token>> = vec![Vec::new()];
    for t in tokens {
        let last_is_option = lines.last().and_then(|l| l.last()).is_some_and(|x| x.kind == Kind::Option);
        if t.kind == Kind::Subcommand && lines.len() == 1 {
            lines[0].push(t.clone());
        } else if t.joined || (t.kind == Kind::Value && last_is_option) {
            lines.last_mut().unwrap().push(t.clone());
        } else if t.kind == Kind::Command {
            lines[0].push(t.clone());
        } else {
            lines.push(vec![t.clone()]);
        }
    }
    lines.into_iter().filter(|l| !l.is_empty()).collect()
}

pub fn line_text(line: &[Token]) -> String {
    let mut out = String::new();
    for t in line {
        if !(t.joined || out.is_empty()) {
            out.push(' ');
        }
        out.push_str(&t.shown);
    }
    out
}

/// Escape for Pango markup (GTK, zenity) and Qt rich text (kdialog).
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

/// Tooltip text, or "" when the system has nothing to say about this part.
pub fn tooltip_markup(token: &Token) -> String {
    token.notes.iter().map(|x| esc(x)).collect::<Vec<_>>().join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install() -> Vec<String> {
        ["install", "-m", "755", "-o", "root", "-g", "root", "/home/u/kod-po/nocne.sh", "/usr/local/bin/nocne.sh"]
            .iter()
            .map(|s| s.to_string())
            .collect()
    }

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    // ShellQuote
    #[test]
    fn quote_plain() {
        assert_eq!(shell_quote("/usr/local/bin/x"), "/usr/local/bin/x");
    }

    #[test]
    fn quote_spaces() {
        assert_eq!(shell_quote("Moje Projekty"), "'Moje Projekty'");
    }

    #[test]
    fn quote_invisible_characters_are_shown() {
        assert_eq!(shell_quote("a\u{200b}b"), "$'a\\u200bb'");
        assert_eq!(shell_quote("a\nb"), "$'a\\nb'");
        assert!(shell_quote("evil\u{202e}txt.sh").contains("\\u202e"));
    }

    #[test]
    fn quote_apostrophe_and_empty() {
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
        assert_eq!(shell_quote(""), "''");
    }

    // Tokenize
    #[test]
    fn install_options_take_values() {
        let argv = install();
        let t = tokenize(&argv, None);
        let kinds: Vec<(Kind, &str)> = t.iter().map(|x| (x.kind, x.arg.as_str())).collect();
        assert_eq!(kinds[..3], [(Kind::Command, "install"), (Kind::Option, "-m"), (Kind::Value, "755")]);
        assert_eq!(kinds[kinds.len() - 2..], [(Kind::Path, argv[7].as_str()), (Kind::Path, argv[8].as_str())]);
    }

    #[test]
    fn role_destination() {
        let t = tokenize(&install(), None);
        let n = t.len();
        assert_eq!((t[n - 2].role, t[n - 1].role), (Role::None, Role::Destination));
    }

    #[test]
    fn long_option_with_equals() {
        let t = tokenize(&v(&["install", "--mode=644", "a", "b"]), None);
        let got: Vec<(Kind, &str, bool)> = t[1..3].iter().map(|x| (x.kind, x.shown.as_str(), x.joined)).collect();
        assert_eq!(got, [(Kind::Option, "--mode=", false), (Kind::Value, "644", true)]);
        assert_eq!(t[2].arg, "644");
    }

    #[test]
    fn attached_short_value() {
        let t = tokenize(&v(&["install", "-m755", "a", "b"]), None);
        assert_eq!((t[2].kind, t[2].arg.as_str()), (Kind::Value, "755"));
    }

    #[test]
    fn subcommand() {
        let t = tokenize(&v(&["systemctl", "restart", "ollama"]), None);
        assert_eq!(t.iter().map(|x| x.kind).collect::<Vec<_>>(), [Kind::Command, Kind::Subcommand, Kind::Arg]);
    }

    #[test]
    fn end_of_options() {
        let t = tokenize(&v(&["rm", "--", "-weird"]), None);
        assert_eq!(t.iter().map(|x| x.kind).collect::<Vec<_>>(), [Kind::Command, Kind::Marker, Kind::Arg]);
    }

    #[test]
    fn value_options_from_man() {
        let extra: HashSet<String> = ["-x".to_string()].into();
        let t = tokenize(&v(&["foo", "-x", "val"]), Some(&extra));
        assert_eq!(t[2].kind, Kind::Value);
    }

    // Layout
    #[test]
    fn short_stays_on_one_line() {
        assert_eq!(layout(&tokenize(&v(&["systemctl", "restart", "ollama"]), None)).len(), 1);
    }

    #[test]
    fn long_is_split_with_values_kept() {
        let argv = install();
        let lines: Vec<String> = layout(&tokenize(&argv, None)).iter().map(|l| line_text(l)).collect();
        assert_eq!(lines, ["install", "-m 755", "-o root", "-g root", argv[7].as_str(), argv[8].as_str()]);
    }

    #[test]
    fn joined_value() {
        let argv = v(&[
            "install",
            "--mode=644",
            &format!("/a/very/long/path/{}", "x".repeat(40)),
            &format!("/another/long/{}", "y".repeat(30)),
        ]);
        let lines: Vec<String> = layout(&tokenize(&argv, None)).iter().map(|l| line_text(l)).collect();
        assert!(lines.contains(&"--mode=644".to_string()));
    }

    #[test]
    fn command_shown_as_typed() {
        // Python: ls shown as "ls" when sudo would find this very file and it is safe; unknown path kept as is
        if let Some(ls) = which("ls", SECURE_PATH) {
            if !unsafe_for_root(&ls) {
                assert_eq!(tokenize(&v(&[&ls, "-la"]), None)[0].shown, "ls");
            }
        }
        assert_eq!(tokenize(&v(&["/no/such/ls"]), None)[0].shown, "/no/such/ls");
    }

    #[test]
    fn tooltip_escapes_markup() {
        let mut t = Token::new("x", Kind::Arg, "x", 0);
        t.notes = vec!["a & <b>".into()];
        assert_eq!(tooltip_markup(&t), "a &amp; &lt;b&gt;");
    }
}
