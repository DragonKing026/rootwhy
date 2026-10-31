// SPDX-License-Identifier: GPL-3.0-or-later
//! The password dialog. `rootwhy` itself never links GTK: the full dialog is `rootwhy-gtk`, a helper next to this
//! binary that gets the dialog data as JSON on stdin and prints the password on stdout. Exit codes of the helper:
//! 0 password, 1 cancelled, anything else (2 = GTK cannot open a display, a dynamic-linker failure…) = unavailable,
//! and the kdialog / zenity fallback is used.

use crate::command::{esc, layout, line_text, tokenize, tooltip_markup, Token};
use crate::explain::explain;
use crate::i18n::{tr, tr_with};
use crate::man::man_info;
use crate::util::{basename, which, SECURE_PATH};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::process::{Command, Stdio};
use zeroize::{Zeroize, Zeroizing};

pub const HELPER_NAME: &str = "rootwhy-gtk";
pub const EXIT_CANCELLED: i32 = 1;
pub const EXIT_NO_GTK: i32 = 2;

/// Variables the dialog process keeps; anything else (GI_*, GIO_*, GTK_*, QT_PLUGIN_PATH, LD_*…) could load
/// foreign code into the process that holds the password.
pub const SAFE_ENV: &[&str] = &[
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "XAUTHORITY",
    "XDG_RUNTIME_DIR",
    "XDG_SESSION_TYPE",
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DESKTOP_SESSION",
    "KDE_FULL_SESSION",
    "KDE_SESSION_VERSION",
    "DBUS_SESSION_BUS_ADDRESS",
    "HOME",
    "USER",
    "LOGNAME",
    "LANG",
    "LANGUAGE",
    "XCURSOR_THEME",
    "XCURSOR_SIZE",
];

/// The environment the dialog (and the helper) runs with: safe variables, LC_*, ROOTWHY_*, and a fixed PATH.
pub fn safe_environ<I: IntoIterator<Item = (String, String)>>(vars: I) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = vars
        .into_iter()
        .filter(|(k, _)| SAFE_ENV.contains(&k.as_str()) || k.starts_with("LC_") || k.starts_with("ROOTWHY_"))
        .collect();
    out.push(("PATH".into(), SECURE_PATH.into()));
    out
}

#[derive(Clone, Default, Debug)]
pub struct Request {
    pub reason: String,
    pub argv: Vec<String>,
    pub attempt: u32,
    pub cwd: String,
    pub caller: String,
    pub preview: bool,
    pub screenshot: String,
}

#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DialogToken {
    pub text: String,
    pub kind: String,
    pub joined: bool,
    /// Pango markup (already escaped), "" when the system has nothing to say
    pub tooltip: String,
}

/// Everything the helper shows, already translated.
#[derive(Serialize, Deserialize, Debug, Clone, Default)]
pub struct DialogData {
    pub title: String,
    pub retry: String,
    pub preview: String,
    pub reason: String,
    pub command_label: String,
    pub lines: Vec<Vec<DialogToken>>,
    pub meta: String,
    pub password_label: String,
    pub cancel_label: String,
    pub authorize_label: String,
    pub screenshot: String,
}

pub fn dialog_title() -> String {
    format!("rootwhy — {}", tr("Administrator access"))
}

fn retry_text(req: &Request) -> String {
    if req.attempt > 1 {
        tr_with("Wrong password — try again (attempt {n}).", &[("n", &req.attempt.to_string())])
    } else {
        String::new()
    }
}

/// Tokens of the request with tooltips from the system.
pub fn explained_tokens(req: &Request) -> Vec<Token> {
    let extra: Option<HashSet<String>> =
        req.argv.first().map(|a| man_info(basename(a)).options.into_iter().filter(|(_, v)| v.0).map(|(k, _)| k).collect());
    let mut tokens = tokenize(&req.argv, extra.as_ref());
    explain(&mut tokens, (!req.cwd.is_empty()).then_some(req.cwd.as_str()), true);
    tokens
}

pub fn build_dialog_data(req: &Request) -> DialogData {
    let tokens = explained_tokens(req);
    let lines = layout(&tokens)
        .iter()
        .map(|line| {
            line.iter()
                .map(|t| DialogToken {
                    text: t.shown.clone(),
                    kind: t.kind.as_str().to_string(),
                    joined: t.joined,
                    tooltip: tooltip_markup(t),
                })
                .collect()
        })
        .collect();
    let mut meta = Vec::new();
    if !req.cwd.is_empty() {
        meta.push(tr_with("Working directory: {d}", &[("d", &req.cwd)]));
    }
    if !req.caller.is_empty() {
        meta.push(tr_with("Requested via: {c} (the programs the request came through)", &[("c", &req.caller)]));
    }
    DialogData {
        title: dialog_title(),
        retry: retry_text(req),
        preview: if req.preview { tr("Preview — nothing will run.") } else { String::new() },
        reason: req.reason.clone(),
        command_label: tr("Command (hover over a part for its description from the system's manual):"),
        lines,
        meta: meta.join("\n"),
        password_label: tr("Password:"),
        cancel_label: tr("Cancel"),
        authorize_label: tr("Authorize"),
        screenshot: req.screenshot.clone(),
    }
}

#[derive(Debug)]
pub enum DialogError {
    /// The GTK helper is missing or cannot start (no libgtk-4, no display…)
    Unavailable,
    Other(String),
}

/// Path of the helper: the same directory as this binary, never PATH.
pub fn helper_path() -> Option<std::path::PathBuf> {
    let me = std::env::current_exe().ok()?;
    let p = me.parent()?.join(HELPER_NAME);
    p.is_file().then_some(p)
}

/// Longest password read from a dialog (sudo itself stops at 255 characters).
pub const PASSWORD_MAX: usize = 4096;

/// Read what a dialog printed into one preallocated buffer that never grows (a growing `Vec` would leave
/// un-wiped copies of the password in freed memory). Input beyond `PASSWORD_MAX` bytes is dropped.
pub fn read_secret<R: Read>(r: &mut R) -> Zeroizing<Vec<u8>> {
    let mut buf = Zeroizing::new(vec![0u8; PASSWORD_MAX]);
    let mut n = 0;
    while n < PASSWORD_MAX {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
    buf.truncate(n); // the bytes past n are still zeros and stay in the same allocation
    buf
}

/// Keys of the variables `safe_environ` would drop.
pub fn unsafe_env_keys<I: IntoIterator<Item = String>>(keys: I) -> Vec<String> {
    keys.into_iter().filter(|k| !(SAFE_ENV.contains(&k.as_str()) || k.starts_with("LC_") || k.starts_with("ROOTWHY_"))).collect()
}

/// Make the environment of this whole process safe (askpass mode), so that no child — man, zenity, kdialog,
/// the helper — gets a variable that loads foreign code (like Python's `_safe_environ`).
/// Called first thing in askpass mode, while the process has a single thread: `remove_var`/`set_var` are
/// not safe next to other threads (they are `unsafe` fns from edition 2024 on).
pub fn sanitize_process_env() {
    for k in unsafe_env_keys(std::env::vars_os().filter_map(|(k, _)| k.into_string().ok())) {
        std::env::remove_var(k);
    }
    // names that are not valid UTF-8 are never in the safe list either
    let bad: Vec<_> = std::env::vars_os().filter(|(k, _)| k.to_str().is_none()).map(|(k, _)| k).collect();
    for k in bad {
        std::env::remove_var(k);
    }
    std::env::set_var("PATH", SECURE_PATH);
}

/// Run the GTK helper. Ok(Some(password)) / Ok(None) = cancelled / Err(Unavailable).
fn run_helper(req: &Request) -> Result<Option<Zeroizing<Vec<u8>>>, DialogError> {
    let helper = helper_path().ok_or(DialogError::Unavailable)?;
    let mut json = Zeroizing::new(serde_json::to_vec(&build_dialog_data(req)).map_err(|e| DialogError::Other(e.to_string()))?);
    let debug = std::env::var_os("ROOTWHY_DEBUG").is_some();
    let mut cmd = Command::new(helper);
    cmd.env_clear().envs(safe_environ(std::env::vars())).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(if debug {
        Stdio::inherit()
    } else {
        Stdio::null()
    }); // theme warnings are noise in the requester's terminal
    let mut child = cmd.spawn().map_err(|_| DialogError::Unavailable)?;
    let mut stdin = child.stdin.take().ok_or(DialogError::Unavailable)?;
    let mut stdout = child.stdout.take().ok_or(DialogError::Unavailable)?;
    let writer = std::thread::spawn(move || {
        let _ = stdin.write_all(&json);
        json.zeroize();
    });
    let out = read_secret(&mut stdout);
    drop(stdout);
    let _ = writer.join();
    let status = child.wait().map_err(|_| DialogError::Unavailable)?;
    match status.code() {
        Some(0) => Ok(Some(out)),
        Some(EXIT_CANCELLED) => Ok(None),
        _ => Err(DialogError::Unavailable),
    }
}

/// kdialog / zenity: no colours or hover, but the same reason and the command split into lines.
pub fn fallback_dialog(req: &Request) -> Result<Option<Zeroizing<Vec<u8>>>, DialogError> {
    let tokens = tokenize(&req.argv, None);
    let raw: Vec<String> = layout(&tokens).iter().map(|l| line_text(l)).collect();
    let n = raw.len();
    let lines: Vec<String> = raw
        .iter()
        .enumerate()
        .map(|(i, line)| format!("{}{}{}", if i > 0 { "    " } else { "" }, line, if i + 1 < n { " \\" } else { "" }))
        .collect();
    let retry = retry_text(req);
    let title = dialog_title();
    let cmd: Vec<String> = if let Some(kdialog) = which("kdialog", SECURE_PATH) {
        let body = lines.iter().map(|x| esc(x).replace("  ", "&nbsp;&nbsp;")).collect::<Vec<_>>().join("<br>");
        let text = format!(
            "{}<p>{}</p><p><code>{}</code></p>",
            if retry.is_empty() { String::new() } else { format!("<p><b>{}</b></p>", esc(&retry)) },
            esc(&req.reason),
            body
        );
        vec![kdialog, "--title".into(), title, "--password".into(), text]
    } else if let Some(zenity) = which("zenity", SECURE_PATH) {
        let text = format!(
            "{}{}\n\n<tt>{}</tt>",
            if retry.is_empty() { String::new() } else { format!("<b>{}</b>\n\n", esc(&retry)) },
            esc(&req.reason),
            lines.iter().map(|x| esc(x)).collect::<Vec<_>>().join("\n")
        );
        vec![
            zenity,
            "--entry".into(),
            "--hide-text".into(),
            "--title".into(),
            title,
            "--width".into(),
            "760".into(),
            "--text".into(),
            text,
        ]
    } else {
        return Err(DialogError::Other(tr("rootwhy: no dialog available (GTK 4, kdialog or zenity)")));
    };
    run_fallback(&cmd, std::env::vars())
}

/// The kdialog / zenity process: cleared environment plus `safe_environ(vars)`, so a user's GTK_MODULES,
/// QT_PLUGIN_PATH, LD_PRELOAD… never reach it. The program itself was found in SECURE_PATH, not the user's PATH.
fn fallback_command<I: IntoIterator<Item = (String, String)>>(cmd: &[String], vars: I) -> Command {
    let mut c = Command::new(&cmd[0]);
    c.args(&cmd[1..]).env_clear().envs(safe_environ(vars)).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null());
    c
}

fn run_fallback<I: IntoIterator<Item = (String, String)>>(
    cmd: &[String],
    vars: I,
) -> Result<Option<Zeroizing<Vec<u8>>>, DialogError> {
    let mut child = fallback_command(cmd, vars).spawn().map_err(|e| DialogError::Other(e.to_string()))?;
    let mut out = child.stdout.take().ok_or_else(|| DialogError::Other("no stdout".into()))?;
    let mut stdout = read_secret(&mut out);
    drop(out);
    let status = child.wait().map_err(|e| DialogError::Other(e.to_string()))?;
    if !status.success() {
        return Ok(None);
    }
    while stdout.last() == Some(&b'\n') {
        stdout.pop();
    }
    Ok(Some(stdout))
}

/// Ask for the password. Ok(None) = cancelled.
pub fn show_dialog(req: &Request) -> Result<Option<Zeroizing<Vec<u8>>>, DialogError> {
    if std::env::var("ROOTWHY_DIALOG").as_deref() != Ok("fallback") {
        match run_helper(req) {
            Err(DialogError::Unavailable) if req.screenshot.is_empty() => {} // no GTK 4 or no display: fall back
            Err(DialogError::Unavailable) => return Err(DialogError::Unavailable),
            other => return other,
        }
    }
    fallback_dialog(req)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_environ_drops_dangerous_variables() {
        let vars = [
            ("PYTHONPATH", "/evil"),
            ("GIO_EXTRA_MODULES", "/evil"),
            ("GTK_PATH", "/evil"),
            ("LD_PRELOAD", "/evil.so"),
            ("QT_PLUGIN_PATH", "/evil"),
            ("LANG", "pl_PL.UTF-8"),
            ("ROOTWHY_REASON", "x"),
            ("LC_TIME", "C"),
            ("PATH", "/home/u/bin"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        let out: std::collections::HashMap<_, _> = safe_environ(vars).into_iter().collect();
        for k in ["PYTHONPATH", "GIO_EXTRA_MODULES", "GTK_PATH", "LD_PRELOAD", "QT_PLUGIN_PATH"] {
            assert!(!out.contains_key(k), "{k}");
        }
        assert_eq!((out["LANG"].as_str(), out["ROOTWHY_REASON"].as_str(), out["LC_TIME"].as_str()), ("pl_PL.UTF-8", "x", "C"));
        assert_eq!(out["PATH"], SECURE_PATH);
    }

    #[test]
    fn fallback_dialog_child_gets_no_dangerous_env() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("rootwhy-fb-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let dump = dir.join("env.txt");
        let fake = dir.join("zenity");
        std::fs::write(&fake, format!("#!/bin/sh\n/usr/bin/env > '{}'\necho secret\n", dump.display())).unwrap();
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        let vars = [
            ("GTK_MODULES", "/tmp/x.so"),
            ("GIO_EXTRA_MODULES", "/tmp/x"),
            ("QT_PLUGIN_PATH", "/tmp/x"),
            ("LD_PRELOAD", "/tmp/x.so"),
            ("DISPLAY", ":0"),
            ("LANG", "C"),
        ]
        .map(|(k, v)| (k.to_string(), v.to_string()));
        // also in the real environment of this process: env_clear must drop what the child would inherit
        std::env::set_var("QT_PLUGIN_PATH", "/tmp/inherited");
        let got = run_fallback(&[fake.to_string_lossy().into_owned()], vars).unwrap().unwrap();
        assert_eq!(&got[..], b"secret");
        let env = std::fs::read_to_string(&dump).unwrap();
        for bad in ["GTK_MODULES", "GIO_EXTRA_MODULES", "QT_PLUGIN_PATH", "LD_PRELOAD"] {
            assert!(!env.contains(bad), "{bad} leaked:\n{env}");
        }
        assert!(env.contains("DISPLAY=:0") && env.contains(&format!("PATH={SECURE_PATH}")), "{env}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unsafe_env_keys_lists_only_dangerous() {
        let keys = ["GTK_MODULES", "LD_PRELOAD", "DISPLAY", "LC_ALL", "ROOTWHY_REASON", "PATH", "HOME", "FOO"].map(String::from);
        let mut bad = unsafe_env_keys(keys);
        bad.sort();
        assert_eq!(bad, ["FOO", "GTK_MODULES", "LD_PRELOAD", "PATH"]);
    }

    #[test]
    fn read_secret_is_bounded_and_does_not_grow() {
        let mut short: &[u8] = b"hunter2\n";
        let s = read_secret(&mut short);
        assert_eq!(&s[..], b"hunter2\n");
        assert_eq!(s.capacity(), PASSWORD_MAX);
        let big = vec![b'a'; PASSWORD_MAX * 2];
        let mut r: &[u8] = &big;
        let s = read_secret(&mut r);
        assert_eq!((s.len(), s.capacity()), (PASSWORD_MAX, PASSWORD_MAX));
    }

    #[test]
    fn dialog_data_roundtrip() {
        crate::i18n::set_lang(crate::i18n::Lang::En);
        let req = Request {
            reason: "Because I need it.".into(),
            argv: vec!["/no/such/tool".into(), "-x".into()],
            attempt: 2,
            cwd: "/tmp".into(),
            caller: "a → b".into(),
            preview: true,
            screenshot: String::new(),
        };
        let data = build_dialog_data(&req);
        let json = serde_json::to_string(&data).unwrap();
        let back: DialogData = serde_json::from_str(&json).unwrap();
        assert_eq!(back.retry, "Wrong password — try again (attempt 2).");
        assert_eq!(back.preview, "Preview — nothing will run.");
        assert_eq!(back.lines.len(), 1);
        assert_eq!(back.lines[0][0].kind, "command");
        assert_eq!(back.meta, "Working directory: /tmp\nRequested via: a → b (the programs the request came through)");
    }
}
