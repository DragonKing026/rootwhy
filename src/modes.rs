// SPDX-License-Identifier: GPL-3.0-or-later
//! The two modes: main mode (parse arguments, write the log, run `sudo -k -A -- command…` with this binary as
//! SUDO_ASKPASS) and askpass mode (started by sudo: show the dialog, print the password to stdout).
//! The password goes from the dialog straight to sudo; the caller (e.g. an AI agent) never sees it.

use crate::cli::{parse_args, REASON_MAX, REASON_MIN};
use crate::command::shell_quote;
use crate::dialog::{safe_environ, sanitize_process_env, show_dialog, DialogError, Request};
use crate::i18n::{tr, tr_with};
use crate::util::{basename, getcwd, home_dir, normpath, realpath, which, which_env, SECURE_PATH};
use std::fs;
use std::io::Write;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use zeroize::Zeroizing;

pub const SUDO_PATHS: [&str; 3] = ["/usr/bin/sudo", "/bin/sudo", "/usr/local/bin/sudo"];

/// The command from the parent's /proc cmdline, only for the exact form rootwhy uses: sudo -k -A -- /abs/command…
/// (an absolute path, so the dialog cannot resolve a bare name differently than sudo does)
pub fn parse_sudo_cmdline(raw: &[u8]) -> Option<Vec<String>> {
    let text = std::str::from_utf8(raw).ok()?;
    let mut parts: Vec<&str> = text.split('\0').collect();
    if parts.last() == Some(&"") {
        parts.pop();
    }
    if parts.len() < 5
        || basename(parts[0]) != "sudo"
        || parts[1..4] != ["-k", "-A", "--"]
        || !parts[4].starts_with('/')
        || normpath(parts[4]) != parts[4]
    {
        return None;
    }
    Some(parts[4..].iter().map(|s| s.to_string()).collect())
}

/// The command the parent sudo will run — None unless the parent really is sudo started by this user.
/// The process name can be faked, so also: real UID is ours, effective UID is 0 (only a setuid-root program gets
/// that), and the executable — when the kernel lets us see it — is a root-owned setuid sudo.
pub fn sudo_parent_command(pid: u32) -> Option<Vec<String>> {
    let proc = PathBuf::from(format!("/proc/{pid}"));
    if fs::read_to_string(proc.join("comm")).ok()?.trim() != "sudo" {
        return None;
    }
    let status = fs::read_to_string(proc.join("status")).ok()?;
    let uid_line = status.lines().find(|l| l.starts_with("Uid:"))?;
    let mut f = uid_line.split_whitespace().skip(1);
    let (real, effective): (u32, u32) = (f.next()?.parse().ok()?, f.next()?.parse().ok()?);
    if real != unsafe { libc::getuid() } || effective != 0 {
        return None;
    }
    match fs::read_link(proc.join("exe")) {
        Ok(exe) => {
            let exe_s = exe.to_str()?;
            let st = fs::metadata(&exe).ok()?;
            if !SUDO_PATHS.contains(&exe_s) || st.uid() != 0 || st.mode() & 0o4000 == 0 {
                return None;
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {} // normal for a setuid process
        Err(_) => return None,
    }
    parse_sudo_cmdline(&fs::read(proc.join("cmdline")).ok()?)
}

/// The programs the request came through, outermost first (at most `depth` levels).
pub fn caller_chain(mut pid: u32, depth: usize) -> String {
    let mut names: Vec<String> = Vec::new();
    for _ in 0..depth {
        let Ok(stat) = fs::read_to_string(format!("/proc/{pid}/stat")) else { break };
        let (Some(a), Some(b)) = (stat.find('('), stat.rfind(')')) else { break };
        names.push(stat[a + 1..b].to_string());
        let Some(ppid) = stat[b + 2..].split_whitespace().nth(1).and_then(|x| x.parse::<u32>().ok()) else { break };
        pid = ppid;
        if pid <= 1 {
            break;
        }
    }
    names.reverse();
    names.join(" → ")
}

fn state_write(path: &str, text: &str) {
    let _ = fs::write(path, text);
}

/// Write all of `buf` to a raw file descriptor (no buffering in between); false on a write error.
pub fn write_fd(fd: libc::c_int, mut buf: &[u8]) -> bool {
    while !buf.is_empty() {
        let n = unsafe { libc::write(fd, buf.as_ptr().cast(), buf.len()) };
        if n < 0 {
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return false;
        }
        buf = &buf[n as usize..];
    }
    true
}

/// Run by sudo -A: show the dialog, print the password. Only when the parent process is sudo — an inherited
/// environment must not turn this into a password prompt for whoever started it.
pub fn askpass_mode() -> i32 {
    let ppid = unsafe { libc::getppid() } as u32;
    let Some(argv) = sudo_parent_command(ppid) else {
        eprintln!("{}", tr("rootwhy: askpass mode is only for sudo -A"));
        return 1;
    };
    // every child from here on (man, zenity, kdialog, the helper) inherits only safe variables; nothing else runs yet
    sanitize_process_env();
    let env: std::collections::HashMap<String, String> = safe_environ(std::env::vars()).into_iter().collect();
    let get = |k: &str| env.get(k).cloned().unwrap_or_default();
    let state = get("ROOTWHY_STATE");
    let mut previous = String::new();
    if !state.is_empty() {
        previous = fs::read_to_string(&state).map(|s| s.trim().to_string()).unwrap_or_default();
    }
    if previous == "cancelled" {
        // sudo asks again after a cancel — no second dialog
        return 1;
    }
    let attempt = if !previous.is_empty() && previous.chars().all(|c| c.is_ascii_digit()) {
        previous.parse::<u32>().unwrap_or(0).saturating_add(1)
    } else {
        1
    };
    if !state.is_empty() {
        state_write(&state, &attempt.to_string());
    }
    // the command shown is the one sudo will run (its own cmdline); the reason is the requester's claim
    let req = Request {
        reason: get("ROOTWHY_REASON"),
        argv,
        attempt,
        cwd: get("ROOTWHY_CWD"),
        caller: get("ROOTWHY_CALLER"),
        ..Default::default()
    };
    match show_dialog(&req) {
        Ok(Some(password)) => {
            // the password is written as it was typed, followed by the newline sudo expects — in one buffer straight
            // to fd 1 (std's stdout keeps a buffered copy that is never wiped)
            let mut line = Zeroizing::new(Vec::with_capacity(password.len() + 1));
            line.extend_from_slice(&password);
            line.push(b'\n');
            write_fd(1, &line);
            0
        }
        Ok(None) => {
            if !state.is_empty() {
                state_write(&state, "cancelled");
                // sudo would print "no password was provided"; it has run nothing yet, so end it quietly — the parent
                // rootwhy reports the cancel (sudo_parent_command checked that the parent is our sudo)
                // only if sudo is still our parent: if it died, its PID may belong to someone else by now
                if unsafe { libc::getppid() } as u32 == ppid {
                    unsafe { libc::kill(ppid as libc::pid_t, libc::SIGKILL) };
                }
            }
            1
        }
        Err(DialogError::Unavailable) => {
            eprintln!("{}", tr("rootwhy: no dialog available (GTK 4, kdialog or zenity)"));
            1
        }
        Err(DialogError::Other(m)) => {
            eprintln!("{m}");
            1
        }
    }
}

/// Collapse control characters to spaces and trim, like the Python `_clean`.
pub fn clean(text: &str) -> String {
    let replaced: String = text.chars().map(|c| if (c as u32) < 0x20 || c as u32 == 0x7f { ' ' } else { c }).collect();
    crate::util::py_strip(&replaced).to_string()
}

fn local_time_parts() -> (String, String) {
    // (YYYYMM, YYYY-MM-DD HH:MM:SS) in local time
    let t = unsafe { libc::time(std::ptr::null_mut()) };
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe { libc::localtime_r(&t, &mut tm) };
    let ym = format!("{:04}{:02}", tm.tm_year + 1900, tm.tm_mon + 1);
    let full = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    );
    (ym, full)
}

fn log(log_dir: &Path, reason: &str, command: &str, status: &str) {
    let result = (|| -> std::io::Result<()> {
        fs::create_dir_all(log_dir)?;
        let (ym, now) = local_time_parts();
        let mut f = fs::OpenOptions::new().create(true).append(true).open(log_dir.join(format!("{ym}.log")))?;
        writeln!(f, "{now} | {reason} | {command} | {status}")
    })();
    if let Err(e) = result {
        eprintln!("rootwhy: log: {e}");
    }
}

/// A fresh private file for the dialog state (mode 0600), like Python's tempfile.mkstemp.
fn make_state_file() -> std::io::Result<String> {
    let dir = std::env::var("XDG_RUNTIME_DIR")
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(|| std::env::var("TMPDIR").ok().filter(|d| !d.is_empty()).unwrap_or_else(|| "/tmp".into()));
    for _ in 0..100 {
        let mut rnd = [0u8; 6];
        fs::File::open("/dev/urandom").and_then(|mut f| std::io::Read::read_exact(&mut f, &mut rnd))?;
        let name: String = rnd.iter().map(|b| format!("{b:02x}")).collect();
        let path = format!("{}/rootwhy.{name}", dir.trim_end_matches('/'));
        match fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(std::io::ErrorKind::AlreadyExists, "no free state file name"))
}

extern "C" fn ignore_sigint(_: libc::c_int) {}

pub fn main_mode(args: &[String]) -> i32 {
    let ns = match parse_args(args) {
        Ok(ns) => ns,
        Err(e) => {
            print!("{}", e.stdout);
            eprint!("{}", e.stderr);
            return e.code;
        }
    };
    let reason = clean(&ns.reason);
    let rlen = reason.chars().count();
    if !(REASON_MIN..=REASON_MAX).contains(&rlen) {
        eprintln!(
            "{}",
            tr_with(
                "rootwhy: reason (-r) is required, {a}–{b} characters",
                &[("a", &REASON_MIN.to_string()), ("b", &REASON_MAX.to_string())]
            )
        );
        return 2;
    }
    if ns.command.is_empty() {
        eprintln!("{}", tr("rootwhy: no command given"));
        return 2;
    }
    let has_display = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if !(has_display("WAYLAND_DISPLAY") || has_display("DISPLAY")) {
        eprintln!("{}", tr("rootwhy: no graphical session for the dialog"));
        return 2;
    }
    let caller = caller_chain(unsafe { libc::getppid() } as u32, 3);
    if ns.preview || !ns.screenshot.is_empty() {
        let req =
            Request { reason, argv: ns.command, attempt: 1, cwd: getcwd(), caller, preview: true, screenshot: ns.screenshot };
        return match show_dialog(&req) {
            Ok(Some(_)) => 0,
            Ok(None) => 130,
            Err(e) => {
                match e {
                    DialogError::Unavailable => eprintln!("{}", tr("rootwhy: no dialog available (GTK 4, kdialog or zenity)")),
                    DialogError::Other(m) => eprintln!("{m}"),
                }
                1
            }
        };
    }

    let mut command = ns.command;
    // resolve once, so the path shown is the path sudo runs (sudo's own PATH may differ from this shell's)
    if !command[0].contains('/') {
        match which(&command[0], SECURE_PATH).or_else(|| which_env(&command[0])) {
            Some(r) => command[0] = r,
            None => {
                eprintln!("{}", tr_with("rootwhy: command not found: {c}", &[("c", &command[0])]));
                return 127;
            }
        }
    } else if !command[0].starts_with('/') || normpath(&command[0]) != command[0] {
        command[0] = realpath(&command[0]); // ./x, a/../b: the file the kernel would really run
    }
    let command_text = command.iter().map(|a| shell_quote(a)).collect::<Vec<_>>().join(" ");
    let log_dir = if !ns.log_dir.is_empty() {
        PathBuf::from(&ns.log_dir)
    } else {
        let base = std::env::var("XDG_STATE_HOME")
            .ok()
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home_dir().unwrap_or_else(|| "/".into())).join(".local/state"));
        base.join("rootwhy")
    };
    log(&log_dir, &reason, &command_text, "start");
    let state = match make_state_file() {
        Ok(s) => s,
        Err(e) => {
            eprintln!("rootwhy: {e}");
            return 1;
        }
    };
    let askpass = std::env::current_exe().ok().map(|p| realpath(&p.to_string_lossy())).unwrap_or_default();
    let sudo = if Path::new("/usr/bin/sudo").exists() { "/usr/bin/sudo" } else { "sudo" };
    let mut cmd = Command::new(sudo);
    cmd.args(["-k", "-A", "--"]).args(&command);
    cmd.env_clear();
    for (k, v) in std::env::vars() {
        if !k.starts_with("ROOTWHY_") && !k.starts_with("SUDO_") {
            cmd.env(k, v);
        }
    }
    cmd.env("ROOTWHY_MODE", "askpass")
        .env("ROOTWHY_STATE", &state)
        .env("ROOTWHY_REASON", &reason)
        .env("ROOTWHY_CWD", getcwd())
        .env("ROOTWHY_CALLER", &caller)
        .env("SUDO_ASKPASS", &askpass);
    // Ctrl-C goes to sudo (same process group); we wait for it and report 130 instead of leaving the state file behind
    unsafe { libc::signal(libc::SIGINT, ignore_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t) };
    use std::os::unix::process::ExitStatusExt;
    let mut code = match cmd.status() {
        Ok(st) => match (st.code(), st.signal()) {
            (Some(c), _) => c,
            (None, Some(libc::SIGINT)) => 130,
            (None, Some(s)) => -s,
            _ => 1,
        },
        Err(e) => {
            eprintln!("rootwhy: {sudo}: {e}");
            127
        }
    };
    let cancelled = fs::read_to_string(&state).map(|s| s.trim() == "cancelled").unwrap_or(false);
    let _ = fs::remove_file(&state);
    if cancelled {
        code = 130;
        eprintln!("{}", tr("rootwhy: cancelled by the user"));
    }
    log(&log_dir, &reason, &command_text, &format!("exit {code}"));
    code & 0xff
}

#[cfg(test)]
mod tests {
    use super::*;

    // Security
    #[test]
    fn sudo_cmdline_exact_form_only() {
        let v = |a: &[&str]| Some(a.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(parse_sudo_cmdline(b"/usr/bin/sudo\0-k\0-A\0--\0/usr/bin/true\0"), v(&["/usr/bin/true"]));
        assert_eq!(parse_sudo_cmdline(b"sudo\0-k\0-A\0--\0/bin/rm\0-rf\0/x\0"), v(&["/bin/rm", "-rf", "/x"]));
        assert_eq!(parse_sudo_cmdline(b"sudo\0-k\0-A\0--\0rm\0/x\0"), None); // bare name
        assert_eq!(parse_sudo_cmdline(b"sudo\0-k\0-A\0--\0/tmp/l/../bin/rm\0"), None); // not normalised
        assert_eq!(parse_sudo_cmdline(b"sudo\0-A\0--\0true\0"), None); // without -k
        assert_eq!(parse_sudo_cmdline(b"sudo\0-k\0-A\0--\0"), None); // no command
        assert_eq!(parse_sudo_cmdline(b"/tmp/fake\0-k\0-A\0--\0true\0"), None); // not sudo
    }

    #[test]
    fn write_fd_writes_everything() {
        use std::io::Read;
        let mut fds = [0 as libc::c_int; 2];
        assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
        write_fd(fds[1], b"secret\n");
        unsafe { libc::close(fds[1]) };
        let mut f = unsafe { <fs::File as std::os::fd::FromRawFd>::from_raw_fd(fds[0]) };
        let mut got = String::new();
        f.read_to_string(&mut got).unwrap();
        assert_eq!(got, "secret\n");
    }

    #[test]
    fn fake_parent_is_rejected() {
        assert_eq!(sudo_parent_command(std::process::id()), None); // comm is the test binary, euid not 0
    }

    #[test]
    fn askpass_refuses_without_sudo_parent() {
        assert_eq!(askpass_mode(), 1);
    }

    #[test]
    fn unsafe_for_root_cases() {
        use crate::util::unsafe_for_root;
        assert!(!unsafe_for_root("/usr/bin/true"));
        let dir = std::env::temp_dir().join(format!("rootwhy-unsafe-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let f = dir.join("s.sh");
        fs::write(&f, "#!/bin/sh\n").unwrap();
        assert!(unsafe_for_root(f.to_str().unwrap())); // owned by this user
        assert!(unsafe_for_root("/nonexistent/xyz")); // errors count as unsafe
        assert!(unsafe_for_root("/tmp/../usr/bin/true")); // ".." counts as unsafe
        let link = dir.join("true");
        std::os::unix::fs::symlink("/usr/bin/true", &link).unwrap();
        assert!(unsafe_for_root(link.to_str().unwrap())); // root-owned target, but a link anyone here can swap
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn log_line_format() {
        let dir = std::env::temp_dir().join(format!("rootwhy-log-{}", std::process::id())).join("sub");
        log(&dir, "reason text", "cmd x", "start");
        log(&dir, "reason text", "cmd x", "exit 0");
        let files: Vec<_> = fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
        assert_eq!(files.len(), 1);
        let name = files[0].file_name().unwrap().to_string_lossy().into_owned();
        assert!(regex::Regex::new(r"^\d{6}\.log$").unwrap().is_match(&name), "{name}");
        let text = fs::read_to_string(&files[0]).unwrap();
        let re = regex::Regex::new(r"^\d{4}-\d\d-\d\d \d\d:\d\d:\d\d \| reason text \| cmd x \| (start|exit 0)$").unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.lines().all(|l| re.is_match(l)), "{text}");
        fs::remove_dir_all(dir.parent().unwrap()).unwrap();
    }

    #[test]
    fn state_file_is_private() {
        let path = make_state_file().unwrap();
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert!(Path::new(&path).file_name().unwrap().to_string_lossy().starts_with("rootwhy."));
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn clean_reason() {
        assert_eq!(clean("  a\tb\nc \u{7f}"), "a b c");
    }

    #[test]
    fn caller_chain_ends_at_init() {
        let c = caller_chain(std::process::id(), 3);
        assert!(!c.is_empty());
    }
}
