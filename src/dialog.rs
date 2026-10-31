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
    let mut out = Zeroizing::new(Vec::new());
    let _ = stdout.read_to_end(&mut out);
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
    let out = Command::new(&cmd[0])
        .args(&cmd[1..])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .output()
        .map_err(|e| DialogError::Other(e.to_string()))?;
    let mut stdout = Zeroizing::new(out.stdout);
    if !out.status.success() {
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
