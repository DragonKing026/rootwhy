// SPDX-License-Identifier: GPL-3.0-or-later
//! Descriptions from the system's man pages (never runs the command itself).

use crate::util::{indent_of, py_isspace, py_strip, splitlines, unsafe_for_root, which, SECURE_PATH};
use regex::Regex;
use std::collections::HashMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

/// {option or subcommand: (takes_value, description)}
pub type ManOptions = HashMap<String, (bool, String)>;

#[derive(Clone, Default, Debug)]
pub struct ManInfo {
    pub summary: String,
    pub options: ManOptions,
}

static CACHE: OnceLock<Mutex<HashMap<String, ManInfo>>> = OnceLock::new();

fn cache() -> &'static Mutex<HashMap<String, ManInfo>> {
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Pre-fill the cache (tests).
pub fn cache_insert(cmd: &str, info: ManInfo) {
    cache().lock().unwrap().insert(cmd.to_string(), info);
}

fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).unwrap())
}

fn clean_man(text: &str) -> String {
    static A: OnceLock<Regex> = OnceLock::new();
    static B: OnceLock<Regex> = OnceLock::new();
    let t = re(&A, r".\x08").replace_all(text, "");
    re(&B, r"\x1b\[[0-9;]*[A-Za-z]").replace_all(&t, "").into_owned()
}

/// One readable sentence: hyphenation joined, first sentence only, at most 160 characters.
fn shorten(description: &str) -> String {
    static A: OnceLock<Regex> = OnceLock::new();
    static B: OnceLock<Regex> = OnceLock::new();
    let d = re(&A, r"‐\s+").replace_all(description, "");
    let d = re(&B, r"\s+").replace_all(&d, " ");
    let d = py_strip(&d).to_string();
    // split at the first  (?<=[a-z\)])\.\s+(?=[A-Z])
    let chars: Vec<(usize, char)> = d.char_indices().collect();
    let mut cut = d.len();
    for (k, &(pos, c)) in chars.iter().enumerate() {
        if c != '.' || k == 0 || !(chars[k - 1].1.is_ascii_lowercase() || chars[k - 1].1 == ')') {
            continue;
        }
        let mut j = k + 1;
        while j < chars.len() && py_isspace(chars[j].1) {
            j += 1;
        }
        if j > k + 1 && j < chars.len() && chars[j].1.is_ascii_uppercase() {
            cut = pos;
            break;
        }
    }
    let mut d = d[..cut].to_string();
    if d.chars().count() > 160 {
        let head: String = d.chars().take(159).collect();
        let head = match head.rfind(' ') {
            Some(i) => head[..i].to_string(),
            None => head,
        };
        d = format!("{head}…");
    }
    d
}

/// From a rendered man page: (one-line summary, {option or subcommand: (takes_value, description)}).
/// A subcommand entry is a lowercase word at the start of an indented line, described by the more-indented lines
/// below it (like `start UNIT...` in systemctl(1)).
pub fn parse_man(text: &str) -> ManInfo {
    static SPLIT1: OnceLock<Regex> = OnceLock::new();
    static SPLIT2: OnceLock<Regex> = OnceLock::new();
    static HEADER: OnceLock<Regex> = OnceLock::new();
    static WORD: OnceLock<Regex> = OnceLock::new();
    static GAP: OnceLock<Regex> = OnceLock::new();
    static NAME: OnceLock<Regex> = OnceLock::new();
    static TAIL: OnceLock<Regex> = OnceLock::new();
    static COMMA: OnceLock<Regex> = OnceLock::new();

    let text = clean_man(text);
    let lines = splitlines(&text);
    let mut summary = String::new();
    for (k, line) in lines.iter().enumerate() {
        let s = py_strip(line);
        if (s == "NAME" || s == "NAZWA") && k + 1 < lines.len() {
            let name_line = lines[k + 1..(k + 3).min(lines.len())]
                .iter()
                .map(|x| py_strip(x))
                .filter(|x| !x.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            // re.split(r"\s+[-–—]\s+", name_line, maxsplit=1)[-1]
            let after = match re(&SPLIT1, r"\s+[-–—]\s+").find(&name_line) {
                Some(m) => name_line[m.end()..].to_string(),
                None => name_line.clone(),
            };
            // re.split(r"\s{2,}|\b[A-Z]{4,}\b", summary)[0]
            let before = match re(&SPLIT2, r"\s{2,}|\b[A-Z]{4,}\b").find(&after) {
                Some(m) => after[..m.start()].to_string(),
                None => after.clone(),
            };
            summary = py_strip(&before).to_string();
            break;
        }
    }

    let header = re(&HEADER, r"^( {2,12})(--?[A-Za-z0-9?].*)$");
    let word = re(&WORD, r"^( {2,12})([a-z][\w-]*)(?:\s+\S.*)?$");
    let mut options: ManOptions = HashMap::new();
    let mut i = 0;
    while i < lines.len() {
        let Some(m) = header.captures(lines[i]) else {
            let w = word.captures(lines[i]);
            let nxt = lines.get(i + 1).copied().unwrap_or("");
            if let Some(w) = w {
                let wlen = w[1].len();
                if !py_strip(nxt).is_empty() && indent_of(nxt) > wlen {
                    let mut desc: Vec<&str> = Vec::new();
                    let mut j = i + 1;
                    while j < lines.len() && !py_strip(lines[j]).is_empty() && indent_of(lines[j]) > wlen {
                        desc.push(py_strip(lines[j]));
                        j += 1;
                    }
                    options.entry(w[2].to_string()).or_insert((false, shorten(&desc.join(" "))));
                    i = j;
                    continue;
                }
            }
            i += 1;
            continue;
        };
        let indent = m[1].len();
        let spec_full = &m[2];
        // description may follow on the same line, after 2+ whitespace characters
        let (spec, rest) = match re(&GAP, r"\s{2,}").find(spec_full) {
            Some(g) => (&spec_full[..g.start()], &spec_full[g.end()..]),
            None => (spec_full, ""),
        };
        let mut desc: Vec<&str> = Vec::new();
        if !py_strip(rest).is_empty() {
            desc.push(py_strip(rest));
        }
        let mut j = i + 1;
        while j < lines.len() && !py_strip(lines[j]).is_empty() && indent_of(lines[j]) > indent {
            desc.push(py_strip(lines[j]));
            j += 1;
        }
        let mut names: Vec<String> = Vec::new();
        let mut takes = false;
        for part in re(&COMMA, r",\s*").split(spec) {
            let Some(nm) = re(&NAME, r"^(--?[A-Za-z0-9?][\w-]*)(.*)$").captures(py_strip(part)) else { continue };
            names.push(nm[1].to_string());
            let tail = &nm[2];
            if !tail.starts_with('[') && re(&TAIL, r"^(=|\s+\S)").is_match(tail) {
                takes = true;
            }
        }
        let d = shorten(&desc.join(" "));
        for n in names {
            options.entry(n).or_insert((takes, d.clone()));
        }
        i = j;
    }
    ManInfo { summary, options }
}

const SYSTEM_MAN_DIRS: [&str; 2] = ["/usr/share/man", "/usr/local/share/man"];

/// Man directories nobody but root can change (the directory and every parent).
fn system_man_dirs() -> Vec<String> {
    SYSTEM_MAN_DIRS.iter().filter(|d| crate::util::is_dir(d) && !unsafe_for_root(d)).map(|d| d.to_string()).collect()
}

/// Run a command with a timeout; returns stdout when it exited with 0.
fn run_with_timeout(cmd: &mut Command, secs: u64) -> Option<Vec<u8>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut buf = Vec::new();
        let _ = stdout.read_to_end(&mut buf);
        buf
    });
    let deadline = Instant::now() + Duration::from_secs(secs);
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break Some(s),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
        }
    };
    let out = reader.join().ok()?;
    status.filter(|s| s.success()).map(|_| out)
}

fn is_man_name(cmd: &str) -> bool {
    !cmd.is_empty() && cmd.chars().all(|c| c.is_alphanumeric() || "_.+-".contains(c))
}

/// Summary and options from `man`. Cached; empty when unavailable.
pub fn man_info(cmd: &str) -> ManInfo {
    if let Some(v) = cache().lock().unwrap().get(cmd) {
        return v.clone();
    }
    let mut result = ManInfo::default();
    let man = which("man", SECURE_PATH);
    let dirs = system_man_dirs();
    if let (true, Some(man), false) = (is_man_name(cmd), man, dirs.is_empty()) {
        // only root-owned system pages, clean environment: the requester runs as this user and could otherwise plant a
        // page (~/.local/share/man, ~/.manpath, MANPATH…) that describes `rm -rf` as harmless
        let mut c = Command::new(man);
        c.env_clear();
        for (k, v) in std::env::vars() {
            if k == "LANG" || k == "LANGUAGE" || k.starts_with("LC_") {
                c.env(k, v);
            }
        }
        // MANPATH rather than -M: with -M man-db skips the translated pages (/usr/share/man/pl…)
        c.env("PATH", SECURE_PATH)
            .env("HOME", "/nonexistent")
            .env("MANPATH", dirs.join(":"))
            .env("MANWIDTH", "400")
            .env("MANPAGER", "cat")
            .env("PAGER", "cat")
            .args(["-C", "/etc/man_db.conf", "-P", "cat", "--", cmd])
            .current_dir("/");
        if let Some(out) = run_with_timeout(&mut c, 4) {
            result = parse_man(&String::from_utf8_lossy(&out));
        }
    }
    cache().lock().unwrap().insert(cmd.to_string(), result.clone());
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAN_SAMPLE: &str = "INSTALL(1)                       User Commands                       INSTALL(1)

NAME
       install - copy files and set attributes

DESCRIPTION
       --backup[=CONTROL]
              make a backup of each existing destination file

       -b     like --backup but does not accept an argument

       -m, --mode=MODE
              set permission mode (as in chmod), instead of rwxr-xr-x

       -t, --target-directory=DIRECTORY
              copy all SOURCE arguments into DIRECTORY
";

    #[test]
    fn parse() {
        let info = parse_man(MAN_SAMPLE);
        assert_eq!(info.summary, "copy files and set attributes");
        let o = &info.options;
        assert!(o["-m"].0);
        assert!(o["--mode"].1.contains("permission mode"));
        assert!(!o["--backup"].0); // optional value does not consume the next argument
        assert!(o["-b"].1.contains("like --backup"));
        assert!(o["-t"].0);
    }

    #[test]
    fn subcommand_entries_and_first_sentence() {
        let info = parse_man(
            "COMMANDS
       start PATTERN...
           Start (activate) one or more units specified on the command line. Note that unit
           names are matched.

       Paragraph text at the same indent
       continues here.
",
        );
        assert_eq!(info.options["start"].1, "Start (activate) one or more units specified on the command line");
        assert!(!info.options.contains_key("Paragraph"));
    }

    #[test]
    fn overstrike_removed() {
        let info = parse_man("NAME\n       f\x08fo\x08oo - does f\x08foo\n");
        assert_eq!(info.summary, "does foo");
    }

    #[test]
    fn long_description_is_cut() {
        let long = format!("--x\n       {}\n", "word ".repeat(60));
        let d = &parse_man(&format!("  {long}")).options["--x"].1;
        assert!(d.ends_with('…') && d.chars().count() <= 160);
    }

    #[test]
    fn man_name_filter() {
        assert!(is_man_name("systemctl"));
        assert!(is_man_name("g++"));
        assert!(!is_man_name("a b"));
        assert!(!is_man_name("../x"));
    }
}
