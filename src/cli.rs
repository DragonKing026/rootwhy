// SPDX-License-Identifier: GPL-3.0-or-later
//! Command-line parsing that behaves like the Python version's argparse parser (abbreviated long options, `-rVALUE`,
//! `--reason=VALUE`, everything after the first non-option argument is the command, `--` ends options).

use crate::i18n::l;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
pub const REASON_MIN: usize = 10;
pub const REASON_MAX: usize = 300;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct Args {
    pub reason: String,
    pub log_dir: String,
    pub preview: bool,
    pub screenshot: String,
    pub command: Vec<String>,
}

/// The parser wants the process to stop: print and exit with `code`.
#[derive(Debug, PartialEq, Eq)]
pub struct Exit {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Option strings in definition order (the order argparse lists them in an ambiguity error).
const OPTION_STRINGS: [&str; 8] = ["-h", "--help", "-r", "--reason", "--log-dir", "--preview", "--screenshot", "--version"];

fn takes_value(opt: &str) -> bool {
    matches!(opt, "-r" | "--reason" | "--log-dir" | "--screenshot")
}

fn display_name(opt: &str) -> &'static str {
    match opt {
        "-h" | "--help" => "-h/--help",
        "-r" | "--reason" => "-r/--reason",
        "--log-dir" => "--log-dir",
        "--preview" => "--preview",
        "--screenshot" => "--screenshot",
        _ => "--version",
    }
}

fn canonical(opt: &str) -> &'static str {
    match opt {
        "-h" | "--help" => "--help",
        "-r" | "--reason" => "--reason",
        "--log-dir" => "--log-dir",
        "--preview" => "--preview",
        "--screenshot" => "--screenshot",
        _ => "--version",
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Class {
    /// An argument (or `--`): starts the command.
    Positional,
    /// A known option with an optional explicit value.
    Opt(&'static str, Option<String>),
    Unknown,
    Ambiguous(Vec<&'static str>),
}

fn is_negative_number(a: &str) -> bool {
    // ^-\d+$|^-\d*\.\d+$
    let Some(rest) = a.strip_prefix('-') else { return false };
    if !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()) {
        return true;
    }
    match rest.split_once('.') {
        Some((i, f)) => i.chars().all(|c| c.is_ascii_digit()) && !f.is_empty() && f.chars().all(|c| c.is_ascii_digit()),
        None => false,
    }
}

fn classify(a: &str) -> Class {
    if a == "--" || !a.starts_with('-') || a == "-" {
        return Class::Positional;
    }
    if let Some(o) = OPTION_STRINGS.iter().find(|o| **o == a) {
        return Class::Opt(o, None);
    }
    if let Some((o, e)) = a.split_once('=') {
        if let Some(o) = OPTION_STRINGS.iter().find(|x| **x == o) {
            return Class::Opt(o, Some(e.to_string()));
        }
    }
    let mut matches: Vec<(&'static str, Option<String>)> = Vec::new();
    if a.starts_with("--") {
        let (prefix, explicit) = match a.split_once('=') {
            Some((p, e)) => (p, Some(e.to_string())),
            None => (a, None),
        };
        for o in OPTION_STRINGS {
            if o.starts_with(prefix) {
                matches.push((o, explicit.clone()));
            }
        }
    } else {
        let short: String = a.chars().take(2).collect();
        let explicit: String = a.chars().skip(2).collect();
        for o in OPTION_STRINGS {
            if o == short {
                matches.push((o, Some(explicit.clone())));
            } else if o.starts_with(a) {
                matches.push((o, None));
            }
        }
    }
    match matches.len() {
        0 => {}
        1 => return Class::Opt(matches[0].0, matches[0].1.clone()),
        _ => return Class::Ambiguous(matches.iter().map(|m| m.0).collect()),
    }
    if is_negative_number(a) || a.contains(' ') {
        return Class::Positional;
    }
    Class::Unknown
}

pub fn term_columns() -> usize {
    if let Some(c) = std::env::var("COLUMNS").ok().and_then(|v| v.parse::<usize>().ok()).filter(|c| *c > 0) {
        return c;
    }
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    if unsafe { libc::ioctl(1, libc::TIOCGWINSZ, &mut ws) } == 0 && ws.ws_col > 0 {
        return ws.ws_col as usize;
    }
    80
}

fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines: Vec<String> = Vec::new();
    let mut cur = String::new();
    for w in text.split_whitespace() {
        let wl = w.chars().count();
        if !cur.is_empty() && cur.chars().count() + 1 + wl > width {
            lines.push(std::mem::take(&mut cur));
        }
        if !cur.is_empty() {
            cur.push(' ');
        }
        cur.push_str(w);
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

pub fn usage_text(columns: usize) -> String {
    let width = columns.saturating_sub(2).max(1);
    let prefix = "usage: ";
    let prog = "rootwhy";
    let opt_parts = ["[-h]", "[-r REASON]", "[--log-dir LOG_DIR]", "[--preview]", "[--version]"];
    let pos_parts = ["..."];
    let one: Vec<&str> = std::iter::once(prog).chain(opt_parts).chain(pos_parts).collect();
    let usage = one.join(" ");
    if prefix.len() + usage.chars().count() <= width {
        return format!("{prefix}{usage}\n");
    }
    let get_lines = |parts: &[&str], indent: &str, with_prefix: bool| -> Vec<String> {
        let mut lines: Vec<String> = Vec::new();
        let mut line: Vec<&str> = Vec::new();
        let mut line_len = if with_prefix { prefix.len() as isize - 1 } else { indent.len() as isize - 1 };
        for part in parts {
            if line_len + 1 + part.chars().count() as isize > width as isize && !line.is_empty() {
                lines.push(format!("{indent}{}", line.join(" ")));
                line.clear();
                line_len = indent.len() as isize - 1;
            }
            line.push(part);
            line_len += part.chars().count() as isize + 1;
        }
        if !line.is_empty() {
            lines.push(format!("{indent}{}", line.join(" ")));
        }
        if with_prefix {
            lines[0] = lines[0][indent.len()..].to_string();
        }
        lines
    };
    let lines = if prefix.len() + prog.len() <= (0.75 * width as f64) as usize {
        let indent = " ".repeat(prefix.len() + prog.len() + 1);
        let mut first: Vec<&str> = vec![prog];
        first.extend(opt_parts);
        let mut lines = get_lines(&first, &indent, true);
        lines.extend(get_lines(&pos_parts, &indent, false));
        lines
    } else {
        let indent = " ".repeat(prefix.len());
        let mut parts: Vec<&str> = opt_parts.to_vec();
        parts.extend(pos_parts);
        let mut lines = get_lines(&parts, &indent, false);
        lines.insert(0, prog.to_string());
        lines
    };
    format!("{prefix}{}\n", lines.join("\n"))
}

pub fn help_text(columns: usize) -> String {
    let width = columns.saturating_sub(2).max(1);
    let desc = l(
        "Run a command as root after a password dialog that shows why, the command in coloured parts and, on hover, each \
         part's description from the system's man pages.",
        "Uruchamia polecenie jako root po haśle w okienku, które pokazuje po co, polecenie w kolorowych częściach i — po \
         najechaniu — opis każdej części z podręcznika systemu (man).",
    );
    let epilog = l(
        "Example: rootwhy -r \"Restart Ollama after changing its service file.\" systemctl restart ollama",
        "Przykład: rootwhy -r \"Restart Ollamy po zmianie pliku usługi.\" systemctl restart ollama",
    );
    let reason_help = l(
        &format!("why the command is needed, {REASON_MIN}–{REASON_MAX} characters (required)"),
        &format!("po co jest polecenie, {REASON_MIN}–{REASON_MAX} znaków (wymagane)"),
    );
    let actions: Vec<(String, String)> = vec![
        ("-h, --help".into(), "show this help message and exit".into()),
        ("-r, --reason REASON".into(), reason_help),
        (
            "--log-dir LOG_DIR".into(),
            l("log directory (default: $XDG_STATE_HOME/rootwhy)", "katalog dziennika (domyślnie $XDG_STATE_HOME/rootwhy)"),
        ),
        ("--preview".into(), l("only show the dialog; run nothing", "tylko pokaż okienko; niczego nie uruchamiaj")),
        ("--version".into(), "show program's version number and exit".into()),
    ];
    let max_len = actions.iter().map(|a| a.0.chars().count()).chain([7]).max().unwrap_or(0) + 2;
    let help_position = (max_len + 2).min(24);
    let help_width = width.saturating_sub(help_position).max(11);
    let action_width = help_position - 2 - 2;

    let mut out = usage_text(columns);
    out.push('\n');
    for line in wrap(&desc, width.max(11)) {
        out.push_str(&line);
        out.push('\n');
    }
    out.push_str("\npositional arguments:\n  command\n\noptions:\n");
    for (inv, help) in &actions {
        let lines = wrap(help, help_width);
        if inv.chars().count() <= action_width {
            out.push_str(&format!("  {:<action_width$}  {}\n", inv, lines.first().cloned().unwrap_or_default()));
            for x in lines.iter().skip(1) {
                out.push_str(&format!("{}{}\n", " ".repeat(help_position), x));
            }
        } else {
            out.push_str(&format!("  {inv}\n"));
            for x in &lines {
                out.push_str(&format!("{}{}\n", " ".repeat(help_position), x));
            }
        }
    }
    out.push('\n');
    for line in wrap(&epilog, width.max(11)) {
        out.push_str(&line);
        out.push('\n');
    }
    out
}

/// Python's repr() of a string, enough for error messages.
fn py_repr(s: &str) -> String {
    let q = if s.contains('\'') && !s.contains('"') { '"' } else { '\'' };
    let mut out = String::from(q);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c == q => {
                out.push('\\');
                out.push(c);
            }
            c if (c as u32) < 0x20 || c as u32 == 0x7f => out.push_str(&format!("\\x{:02x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push(q);
    out
}

fn error(columns: usize, msg: &str) -> Exit {
    Exit { code: 2, stdout: String::new(), stderr: format!("{}rootwhy: error: {msg}\n", usage_text(columns)) }
}

pub fn parse_args(args: &[String]) -> Result<Args, Exit> {
    parse_args_cols(args, term_columns())
}

pub fn parse_args_cols(args: &[String], columns: usize) -> Result<Args, Exit> {
    // argparse classifies every argument before the first `--` up front: an ambiguous one fails right away
    for a in args.iter().take_while(|a| *a != "--") {
        if let Class::Ambiguous(list) = classify(a) {
            return Err(error(columns, &format!("ambiguous option: {a} could match {}", list.join(", "))));
        }
    }
    let mut out = Args { log_dir: std::env::var("ROOTWHY_LOG_DIR").unwrap_or_default(), ..Default::default() };
    let mut extras: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        match classify(a) {
            Class::Positional => {
                out.command = args[i..].to_vec();
                break;
            }
            Class::Unknown => extras.push(a.clone()),
            Class::Ambiguous(_) => unreachable!("handled above"),
            Class::Opt(opt, explicit) => {
                let name = display_name(opt);
                match canonical(opt) {
                    "--help" if explicit.is_none() || !opt.starts_with("--") => {
                        return Err(Exit { code: 0, stdout: help_text(columns), stderr: String::new() })
                    }
                    "--version" if explicit.is_none() => {
                        return Err(Exit { code: 0, stdout: format!("rootwhy {VERSION}\n"), stderr: String::new() })
                    }
                    "--help" | "--version" | "--preview" if explicit.is_some() => {
                        return Err(error(
                            columns,
                            &format!("argument {name}: ignored explicit argument {}", py_repr(explicit.as_deref().unwrap_or(""))),
                        ));
                    }
                    "--preview" => out.preview = true,
                    _ => {
                        debug_assert!(takes_value(opt));
                        let value = match explicit {
                            Some(e) => e,
                            None => match args.get(i + 1) {
                                Some(next) if classify(next) == Class::Positional && next != "--" => {
                                    i += 1;
                                    next.clone()
                                }
                                _ => return Err(error(columns, &format!("argument {name}: expected one argument"))),
                            },
                        };
                        match canonical(opt) {
                            "--reason" => out.reason = value,
                            "--log-dir" => out.log_dir = value,
                            _ => out.screenshot = value,
                        }
                    }
                }
            }
        }
        i += 1;
    }
    if !extras.is_empty() {
        return Err(error(columns, &format!("unrecognized arguments: {}", extras.join(" "))));
    }
    if out.command.first().map(String::as_str) == Some("--") {
        out.command.remove(0);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(a: &[&str]) -> Result<Args, Exit> {
        let v: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        parse_args_cols(&v, 80)
    }

    fn ok(a: &[&str]) -> (String, bool, Vec<String>) {
        let r = p(a).unwrap();
        (r.reason, r.preview, r.command)
    }

    fn err(a: &[&str]) -> String {
        let e = p(a).unwrap_err();
        assert_eq!(e.code, 2);
        e.stderr.lines().last().unwrap().to_string()
    }

    // Expected values below come from the Python 3.14 argparse parser of rootwhy 2.0.0.
    #[test]
    fn like_python_parser() {
        let c = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(ok(&["-r", "reason text here", "true"]), ("reason text here".into(), false, c(&["true"])));
        assert_eq!(ok(&["-r", "x", "--", "true", "-a"]), ("x".into(), false, c(&["true", "-a"])));
        assert_eq!(ok(&["--reason=abc", "cmd", "-r", "z"]), ("abc".into(), false, c(&["cmd", "-r", "z"])));
        assert_eq!(ok(&["--rea", "abc", "--prev", "c"]), ("abc".into(), true, c(&["c"])));
        assert_eq!(ok(&["-rabc", "c"]), ("abc".into(), false, c(&["c"])));
        assert_eq!(ok(&["-r=abc", "c"]), ("abc".into(), false, c(&["c"])));
        assert_eq!(ok(&["-r", "-5", "c"]), ("-5".into(), false, c(&["c"])));
        assert_eq!(ok(&["-r", "-x y", "c"]), ("-x y".into(), false, c(&["c"])));
        assert_eq!(ok(&["cmd", "-r", "x"]), (String::new(), false, c(&["cmd", "-r", "x"])));
        assert_eq!(ok(&["--", "--", "a"]), (String::new(), false, c(&["--", "a"])));
        assert_eq!(ok(&[]), (String::new(), false, vec![]));
        assert_eq!(ok(&["--preview", "--preview", "c"]), (String::new(), true, c(&["c"])));
        assert_eq!(ok(&["-", "c"]), (String::new(), false, c(&["-", "c"])));
        assert_eq!(ok(&["", "c"]), (String::new(), false, c(&["", "c"])));
        assert_eq!(ok(&["-r", "x", "-3", "c"]), ("x".into(), false, c(&["-3", "c"])));
        assert_eq!(ok(&["-r", "-"]), ("-".into(), false, vec![]));
        assert_eq!(ok(&["--r=", "c"]), (String::new(), false, c(&["c"])));
        let r = p(&["--log", "/x", "c"]).unwrap();
        assert_eq!((r.log_dir.as_str(), r.command), ("/x", c(&["c"])));
        let r = p(&["--screenshot", "a.png", "c"]).unwrap();
        assert_eq!(r.screenshot, "a.png");
    }

    #[test]
    fn errors_like_python_parser() {
        assert_eq!(err(&["-r"]), "rootwhy: error: argument -r/--reason: expected one argument");
        assert_eq!(err(&["-r", "-x"]), "rootwhy: error: argument -r/--reason: expected one argument");
        assert_eq!(err(&["-r", "--", "foo", "true"]), "rootwhy: error: argument -r/--reason: expected one argument");
        assert_eq!(err(&["--reason", "--preview", "c"]), "rootwhy: error: argument -r/--reason: expected one argument");
        assert_eq!(err(&["--screenshot"]), "rootwhy: error: argument --screenshot: expected one argument");
        assert_eq!(err(&["--preview=1"]), "rootwhy: error: argument --preview: ignored explicit argument '1'");
        assert_eq!(err(&["--prev=1", "c"]), "rootwhy: error: argument --preview: ignored explicit argument '1'");
        assert_eq!(err(&["-x", "c"]), "rootwhy: error: unrecognized arguments: -x");
        assert_eq!(err(&["-e", "true=x", "true"]), "rootwhy: error: unrecognized arguments: -e");
        assert_eq!(
            err(&["--=x"]),
            "rootwhy: error: ambiguous option: --=x could match --help, --reason, --log-dir, --preview, --screenshot, --version"
        );
    }

    #[test]
    fn help_and_version() {
        for a in [&["--version", "-x"][..], &["-x", "--version"], &["--vers"]] {
            let e = p(a).unwrap_err();
            assert_eq!((e.code, e.stdout.as_str()), (0, "rootwhy 3.0.0\n"));
        }
        for a in [&["-h", "-x"][..], &["-hz"], &["--he"]] {
            let e = p(a).unwrap_err();
            assert_eq!(e.code, 0);
            assert!(e.stdout.starts_with("usage: rootwhy [-h]"));
        }
    }

    #[test]
    fn usage_wraps_like_python_at_80_columns() {
        assert_eq!(
            usage_text(80),
            "usage: rootwhy [-h] [-r REASON] [--log-dir LOG_DIR] [--preview] [--version]\n               ...\n"
        );
        assert_eq!(usage_text(200), "usage: rootwhy [-h] [-r REASON] [--log-dir LOG_DIR] [--preview] [--version] ...\n");
    }
}
