// SPDX-License-Identifier: GPL-3.0-or-later
//! Tooltips from the system only: man pages (in the system's language, when translated) and checks of the files
//! involved. A part the system says nothing about gets no tooltip.

use crate::command::{Kind, Role, Token};
use crate::i18n::tr;
use crate::man::man_info;
use crate::util::{
    abspath, basename, expanduser, getcwd, is_dir, is_file, is_symlink, join, lexists, parent_of_abs, unsafe_for_root, which,
    SECURE_PATH,
};
use std::ffi::CString;

pub fn mode_warning(value: &str) -> Option<String> {
    let octal = (3..=4).contains(&value.len()) && value.chars().all(|c| ('0'..='7').contains(&c));
    if octal && u32::from_str_radix(value, 8).ok()? & 0o6000 != 0 {
        return Some(tr("⚠ setuid/setgid bit — runs with the owner's rights"));
    }
    None
}

fn user_exists(user: &str) -> bool {
    if !user.is_empty() && user.chars().all(|c| c.is_ascii_digit()) {
        return user.parse::<u32>().map(|u| !unsafe { libc::getpwuid(u) }.is_null()).unwrap_or(false);
    }
    CString::new(user).map(|c| !unsafe { libc::getpwnam(c.as_ptr()) }.is_null()).unwrap_or(false)
}

fn group_exists(group: &str) -> bool {
    if !group.is_empty() && group.chars().all(|c| c.is_ascii_digit()) {
        return group.parse::<u32>().map(|g| !unsafe { libc::getgrgid(g) }.is_null()).unwrap_or(false);
    }
    CString::new(group).map(|c| !unsafe { libc::getgrnam(c.as_ptr()) }.is_null()).unwrap_or(false)
}

pub fn owner_warning(value: &str) -> Option<String> {
    let (user, group) = match value.split_once(':') {
        Some((u, g)) => (u, g),
        None => (value, ""),
    };
    if (!user.is_empty() && !user_exists(user)) || (!group.is_empty() && !group_exists(group)) {
        return Some(tr("⚠ no such user or group on this system"));
    }
    None
}

/// Only what deserves attention: a missing path, a destination that gets replaced, an unsafe script.
pub fn path_warnings(path: &str, cwd: Option<&str>, role: Role) -> Vec<String> {
    let base = cwd.filter(|c| !c.is_empty()).map(str::to_string).unwrap_or_else(getcwd);
    let p = join(&base, &expanduser(path));
    if !lexists(&p) {
        let parent = parent_of_abs(&p);
        return if is_dir(&parent) {
            vec![if role == Role::Destination { tr("does not exist yet (will be created)") } else { tr("⚠ does not exist") }]
        } else {
            vec![tr("⚠ does not exist, and neither does its parent directory")]
        };
    }
    if role == Role::Destination && is_file(&p) && !is_symlink(&p) {
        return vec![tr("⚠ an existing file — it will be replaced")];
    }
    if role == Role::Script && unsafe_for_root(&p) {
        return vec![tr("⚠ not owned by root or writable by others — could be changed before it runs")];
    }
    Vec::new()
}

fn resolve_command(cmd: &str) -> Option<String> {
    if cmd.contains('/') {
        return Some(abspath(cmd));
    }
    which(cmd, SECURE_PATH)
}

/// Fill token.notes from the system only.
pub fn explain(tokens: &mut [Token], cwd: Option<&str>, use_system: bool) {
    if tokens.is_empty() {
        return;
    }
    let cmd = basename(&tokens[0].arg).to_string();
    let info = if use_system { man_info(&cmd) } else { Default::default() };
    let entry = |name: &str| info.options.get(name).map(|e| e.1.clone()).unwrap_or_default();
    for t in tokens.iter_mut() {
        match t.kind {
            Kind::Command => {
                if !info.summary.is_empty() {
                    t.notes.push(info.summary.clone());
                }
                if use_system {
                    match resolve_command(&t.arg) {
                        None => t.notes.push(tr("⚠ not found in sudo's PATH")),
                        Some(resolved) if unsafe_for_root(&resolved) => {
                            t.notes.push(resolved);
                            t.notes.push(tr("⚠ not owned by root or writable by others — could be changed before it runs"));
                        }
                        Some(_) => {}
                    }
                }
            }
            Kind::Subcommand | Kind::Option => {
                let key = if t.option.is_empty() { t.arg.clone() } else { t.option.clone() };
                let text = entry(&key);
                if !text.is_empty() {
                    t.notes.push(text);
                } else if t.kind == Kind::Option && t.option.chars().count() > 2 && !t.option.starts_with("--") {
                    // -la → -l, -a
                    for ch in t.option.chars().skip(1) {
                        let d = entry(&format!("-{ch}"));
                        if !d.is_empty() {
                            t.notes.push(format!("-{ch}  {d}"));
                        }
                    }
                }
            }
            _ if t.role == Role::Mode => t.notes.extend(mode_warning(&t.arg)),
            _ if t.role == Role::Owner && use_system => t.notes.extend(owner_warning(&t.arg)),
            Kind::Value if matches!(t.option.as_str(), "-m" | "--mode") && matches!(cmd.as_str(), "install" | "mkdir") => {
                t.notes.extend(mode_warning(&t.arg))
            }
            Kind::Value if matches!(t.option.as_str(), "-o" | "--owner" | "-g" | "--group") && cmd == "install" && use_system => {
                let v = if matches!(t.option.as_str(), "-g" | "--group") { format!(":{}", t.arg) } else { t.arg.clone() };
                t.notes.extend(owner_warning(&v));
            }
            Kind::Path if use_system => t.notes.extend(path_warnings(&t.arg, cwd, t.role)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{tokenize, tooltip_markup};
    use crate::i18n::{set_lang, Lang};
    use crate::man::{cache_insert, ManInfo};
    use std::collections::HashMap;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn only_system_sources() {
        set_lang(Lang::En);
        let mut options = HashMap::new();
        options.insert("-l".to_string(), (false, "long format".to_string()));
        options.insert("-a".to_string(), (false, "all files".to_string()));
        cache_insert("fakecmd", ManInfo { summary: "does fake things".into(), options });
        let mut toks = tokenize(&v(&["fakecmd", "-la", "plain"]), None);
        explain(&mut toks, None, true);
        assert_eq!(toks[0].notes[0], "does fake things");
        assert_eq!(toks[1].notes, ["-l  long format", "-a  all files"]);
        assert!(toks[2].notes.is_empty()); // the system knows nothing -> no tooltip
        assert_eq!(tooltip_markup(&toks[2]), "");
    }

    #[test]
    fn mode_and_owner_warnings() {
        set_lang(Lang::En);
        assert!(mode_warning("4755").unwrap().contains("setuid"));
        assert!(mode_warning("755").is_none());
        assert!(mode_warning("u+x").is_none());
        assert!(owner_warning("root:root").is_none());
        assert!(owner_warning("no-such-user-xyz").unwrap().contains("no such user"));
    }

    #[test]
    fn path_warnings_cases() {
        set_lang(Lang::En);
        let dir = std::env::temp_dir().join(format!("rootwhy-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("f.txt");
        std::fs::write(&f, "x").unwrap();
        let fs_ = f.to_str().unwrap();
        let new = dir.join("new");
        let news = new.to_str().unwrap();
        assert!(path_warnings(fs_, None, Role::None).is_empty()); // exists, nothing to say
        assert!(path_warnings(fs_, None, Role::Destination)[0].contains("will be replaced"));
        assert_eq!(path_warnings(news, None, Role::Destination), ["does not exist yet (will be created)"]);
        assert_eq!(path_warnings(news, None, Role::None), ["⚠ does not exist"]);
        assert!(path_warnings(fs_, None, Role::Script)[0].contains("could be changed"));
        assert_eq!(
            path_warnings(dir.join("no/such/dir/x").to_str().unwrap(), None, Role::None),
            ["⚠ does not exist, and neither does its parent directory"]
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
