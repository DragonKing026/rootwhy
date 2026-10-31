// SPDX-License-Identifier: GPL-3.0-or-later
//! Small helpers that mirror the Python standard-library behaviour the original relies on
//! (`os.path.*`, `shutil.which`, `str.isspace`…), plus the "could someone else replace this file" check.

use std::ffi::CString;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};

pub const SECURE_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"; // sudo's default secure_path

/// Python's `str.isspace` for one character.
pub fn py_isspace(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

pub fn py_strip(s: &str) -> &str {
    s.trim_matches(py_isspace)
}

/// Python's `str.splitlines()` (without keepends).
pub fn splitlines(s: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut start = 0;
    let mut chars = s.char_indices().peekable();
    while let Some((i, c)) = chars.next() {
        let is_break = matches!(c, '\n' | '\r' | '\u{0b}' | '\u{0c}' | '\u{1c}' | '\u{1d}' | '\u{1e}' | '\u{85}' | '\u{2028}' | '\u{2029}');
        if is_break {
            out.push(&s[start..i]);
            let mut end = i + c.len_utf8();
            if c == '\r' {
                if let Some(&(j, '\n')) = chars.peek() {
                    chars.next();
                    end = j + 1;
                }
            }
            start = end;
        }
    }
    if start < s.len() {
        out.push(&s[start..]);
    }
    out
}

/// Number of leading whitespace characters.
pub fn indent_of(s: &str) -> usize {
    s.chars().take_while(|c| py_isspace(*c)).count()
}

pub fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or("")
}

/// `os.path.normpath` (POSIX).
pub fn normpath(path: &str) -> String {
    if path.is_empty() {
        return ".".into();
    }
    let mut initial = if path.starts_with('/') { 1 } else { 0 };
    if path.starts_with("//") && !path.starts_with("///") {
        initial = 2;
    }
    let mut comps: Vec<&str> = Vec::new();
    for comp in path.split('/') {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp != ".." || (initial == 0 && comps.is_empty()) || comps.last() == Some(&"..") {
            comps.push(comp);
        } else if !comps.is_empty() {
            comps.pop();
        }
    }
    let joined = format!("{}{}", "/".repeat(initial), comps.join("/"));
    if joined.is_empty() {
        ".".into()
    } else {
        joined
    }
}

pub fn getcwd() -> String {
    std::env::current_dir().map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| "/".into())
}

/// `os.path.join(a, b)`: an absolute `b` replaces `a`.
pub fn join(a: &str, b: &str) -> String {
    if b.starts_with('/') || a.is_empty() {
        b.to_string()
    } else if a.ends_with('/') {
        format!("{a}{b}")
    } else {
        format!("{a}/{b}")
    }
}

pub fn abspath(path: &str) -> String {
    normpath(&join(&getcwd(), path))
}

/// `os.path.realpath` (non-strict): symlinks resolved as far as the path exists, the rest kept.
pub fn realpath(path: &str) -> String {
    let abs = join(&getcwd(), path);
    let mut pending: std::collections::VecDeque<String> = abs.split('/').map(str::to_string).collect();
    let mut stack: Vec<String> = Vec::new();
    let mut links = 0;
    while let Some(comp) = pending.pop_front() {
        if comp.is_empty() || comp == "." {
            continue;
        }
        if comp == ".." {
            stack.pop();
            continue;
        }
        let candidate = format!("/{}", stack.iter().cloned().chain(std::iter::once(comp.clone())).collect::<Vec<_>>().join("/"));
        match fs::symlink_metadata(&candidate) {
            Ok(m) if m.file_type().is_symlink() => {
                links += 1;
                if links > 40 {
                    // loop: give up resolving, keep the rest as it is
                    stack.push(comp);
                    stack.extend(pending.drain(..).filter(|c| !c.is_empty()));
                    break;
                }
                match fs::read_link(&candidate) {
                    Ok(target) => {
                        let t = target.to_string_lossy().into_owned();
                        if t.starts_with('/') {
                            stack.clear();
                        }
                        for c in t.split('/').rev() {
                            pending.push_front(c.to_string());
                        }
                    }
                    Err(_) => stack.push(comp),
                }
            }
            _ => stack.push(comp),
        }
    }
    format!("/{}", stack.join("/"))
}

pub fn lexists(p: &str) -> bool {
    fs::symlink_metadata(p).is_ok()
}

pub fn is_dir(p: &str) -> bool {
    fs::metadata(p).map(|m| m.is_dir()).unwrap_or(false)
}

pub fn is_file(p: &str) -> bool {
    fs::metadata(p).map(|m| m.is_file()).unwrap_or(false)
}

pub fn is_symlink(p: &str) -> bool {
    fs::symlink_metadata(p).map(|m| m.file_type().is_symlink()).unwrap_or(false)
}

fn dirname(p: &str) -> &str {
    match p.rfind('/') {
        None => "",
        Some(i) => {
            let head = &p[..=i];
            if head.chars().all(|c| c == '/') {
                head
            } else {
                head.trim_end_matches('/')
            }
        }
    }
}

pub fn parent_of_abs(p: &str) -> String {
    let d = dirname(&abspath(p)).to_string();
    if d.is_empty() {
        "/".into()
    } else {
        d
    }
}

fn access_x(p: &str) -> bool {
    CString::new(p).map(|c| unsafe { libc::access(c.as_ptr(), libc::X_OK) == 0 }).unwrap_or(false)
}

/// `shutil.which(cmd, path=path)`.
pub fn which(cmd: &str, path: &str) -> Option<String> {
    let ok = |f: &str| fs::metadata(f).map(|m| !m.is_dir()).unwrap_or(false) && access_x(f);
    if cmd.contains('/') {
        return ok(cmd).then(|| cmd.to_string());
    }
    for dir in path.split(':') {
        let f = join(dir, cmd);
        if ok(&f) {
            return Some(f);
        }
    }
    None
}

/// `shutil.which(cmd)` with the process's own PATH.
pub fn which_env(cmd: &str) -> Option<String> {
    let path = std::env::var("PATH").unwrap_or_else(|_| "/bin:/usr/bin".into());
    which(cmd, &path)
}

pub fn home_dir() -> Option<String> {
    if let Ok(h) = std::env::var("HOME") {
        if !h.is_empty() {
            return Some(h);
        }
    }
    pw_dir_of_uid(unsafe { libc::getuid() })
}

fn cstr_to_string(p: *const libc::c_char) -> String {
    unsafe { std::ffi::CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

fn pw_dir_of_uid(uid: u32) -> Option<String> {
    let pw = unsafe { libc::getpwuid(uid) };
    (!pw.is_null()).then(|| cstr_to_string(unsafe { (*pw).pw_dir }))
}

/// `os.path.expanduser`.
pub fn expanduser(path: &str) -> String {
    if !path.starts_with('~') {
        return path.to_string();
    }
    let i = path.find('/').unwrap_or(path.len());
    let name = &path[1..i];
    let home = if name.is_empty() {
        home_dir()
    } else {
        CString::new(name).ok().and_then(|c| {
            let pw = unsafe { libc::getpwnam(c.as_ptr()) };
            (!pw.is_null()).then(|| cstr_to_string(unsafe { (*pw).pw_dir }))
        })
    };
    match home {
        Some(h) => {
            let r = format!("{}{}", h.trim_end_matches('/'), &path[i..]);
            if r.is_empty() {
                "/".into()
            } else {
                r
            }
        }
        None => path.to_string(),
    }
}

/// True when someone other than root could replace what runs: the file or any directory on its real path is not
/// owned by root, or is writable by group/others (a sticky directory such as /tmp counts only for its own entries'
/// owners). Errors count as unsafe.
pub fn unsafe_for_root(path: &str) -> bool {
    if Path::new(path).components().any(|c| matches!(c, Component::ParentDir)) {
        // abspath would cancel "link/.." lexically, the kernel would follow the link
        return true;
    }
    const IWGRP: u32 = 0o020;
    const IWOTH: u32 = 0o002;
    const ISVTX: u32 = 0o1000;
    // both the path as given (its symlinks and their directories) and where the symlinks lead
    for target in [PathBuf::from(abspath(path)), PathBuf::from(realpath(path))] {
        for part in target.ancestors() {
            let Ok(st) = fs::symlink_metadata(part) else { return true };
            if st.file_type().is_symlink() {
                if st.uid() != 0 {
                    return true;
                }
                continue; // a symlink's own mode bits mean nothing; its directory is checked next
            }
            let mut writable = st.mode() & (IWGRP | IWOTH) != 0;
            if part != target && st.is_dir() && st.mode() & ISVTX != 0 {
                writable = false; // sticky: others cannot replace entries they do not own
            }
            if st.uid() != 0 || writable {
                return true;
            }
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normpath_cases() {
        assert_eq!(normpath("/a/./b//c/.."), "/a/b");
        assert_eq!(normpath("//a"), "//a");
        assert_eq!(normpath("///a"), "/a");
        assert_eq!(normpath("a/../.."), "..");
        assert_eq!(normpath(""), ".");
        assert_eq!(normpath("/.."), "/");
    }

    #[test]
    fn splitlines_like_python() {
        assert_eq!(splitlines("a\nb\r\nc\rd"), vec!["a", "b", "c", "d"]);
        assert_eq!(splitlines("a\n"), vec!["a"]);
        assert_eq!(splitlines(""), Vec::<&str>::new());
        assert_eq!(splitlines("a\n\nb"), vec!["a", "", "b"]);
    }

    #[test]
    fn realpath_keeps_missing_tail() {
        assert_eq!(realpath("/nonexistent/x/../y"), "/nonexistent/y");
    }
}
