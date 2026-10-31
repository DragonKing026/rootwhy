// SPDX-License-Identifier: GPL-3.0-or-later
//! Language selection and the Polish catalogue (the English text is the key, as in the Python version).

use std::cell::Cell;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Lang {
    En,
    Pl,
}

/// First non-empty of ROOTWHY_LANG, LANGUAGE, LC_ALL, LC_MESSAGES, LANG decides; none -> English.
pub fn detect_lang(get: impl Fn(&str) -> Option<String>) -> Lang {
    for var in ["ROOTWHY_LANG", "LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"] {
        if let Some(value) = get(var).filter(|v| !v.is_empty()) {
            let first = value.split(':').next().unwrap_or("").to_lowercase();
            return if first.starts_with("pl") { Lang::Pl } else { Lang::En };
        }
    }
    Lang::En
}

thread_local! {
    static LANG: Cell<Option<Lang>> = const { Cell::new(None) };
}

pub fn lang() -> Lang {
    LANG.with(|l| {
        if let Some(v) = l.get() {
            v
        } else {
            let v = detect_lang(|k| std::env::var(k).ok());
            l.set(Some(v));
            v
        }
    })
}

/// Override the language for the current thread (tests).
pub fn set_lang(lang: Lang) {
    LANG.with(|l| l.set(Some(lang)));
}

/// Pick by language.
pub fn l(en: &str, pl: &str) -> String {
    if lang() == Lang::Pl { pl } else { en }.to_string()
}

pub const PL: &[(&str, &str)] = &[
    ("Administrator access", "Uprawnienia administratora"),
    (
        "Command (hover over a part for its description from the system's manual):",
        "Polecenie (najedź na fragment, żeby zobaczyć opis z podręcznika systemu):",
    ),
    ("Password:", "Hasło:"),
    ("Cancel", "Anuluj"),
    ("Authorize", "Zatwierdź"),
    ("Wrong password — try again (attempt {n}).", "Błędne hasło — spróbuj ponownie (próba {n})."),
    ("Working directory: {d}", "Katalog roboczy: {d}"),
    (
        "Requested via: {c} (the programs the request came through)",
        "Kto prosi: {c} (programy, przez które przyszła prośba)",
    ),
    ("Preview — nothing will run.", "Podgląd — nic się nie wykona."),
    ("does not exist yet (will be created)", "jeszcze nie istnieje (zostanie utworzony)"),
    ("⚠ does not exist", "⚠ nie istnieje"),
    (
        "⚠ does not exist, and neither does its parent directory",
        "⚠ nie istnieje, nie ma też katalogu nadrzędnego",
    ),
    ("⚠ not found in sudo's PATH", "⚠ nie ma go w PATH sudo"),
    (
        "⚠ not owned by root or writable by others — could be changed before it runs",
        "⚠ właścicielem nie jest root albo inni mogą go zmieniać — może zostać podmieniony przed uruchomieniem",
    ),
    ("⚠ an existing file — it will be replaced", "⚠ plik istnieje — zostanie zastąpiony"),
    ("⚠ no such user or group on this system", "⚠ nie ma takiego użytkownika lub grupy w tym systemie"),
    (
        "⚠ setuid/setgid bit — runs with the owner's rights",
        "⚠ bit setuid/setgid — uruchamia się z prawami właściciela",
    ),
    (
        "rootwhy: reason (-r) is required, {a}–{b} characters",
        "rootwhy: powód (-r) jest wymagany, {a}–{b} znaków",
    ),
    ("rootwhy: no command given", "rootwhy: brak polecenia"),
    ("rootwhy: no graphical session for the dialog", "rootwhy: brak sesji graficznej na okienko"),
    ("rootwhy: cancelled by the user", "rootwhy: anulowane przez użytkownika"),
    ("rootwhy: askpass mode is only for sudo -A", "rootwhy: tryb hasła tylko dla sudo -A"),
    ("rootwhy: command not found: {c}", "rootwhy: nie ma polecenia: {c}"),
    (
        "rootwhy: no dialog available (GTK 4, kdialog or zenity)",
        "rootwhy: brak okienka (GTK 4, kdialog lub zenity)",
    ),
];

/// Translate a catalogue text (no placeholders).
pub fn tr(text: &str) -> String {
    if lang() == Lang::Pl {
        if let Some((_, pl)) = PL.iter().find(|(en, _)| *en == text) {
            return (*pl).to_string();
        }
    }
    text.to_string()
}

/// Translate and fill `{name}` placeholders.
pub fn tr_with(text: &str, args: &[(&str, &str)]) -> String {
    let mut out = tr(text);
    for (k, v) in args {
        out = out.replace(&format!("{{{k}}}"), v);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn placeholders(s: &str) -> Vec<String> {
        let mut v: Vec<String> = regex::Regex::new(r"\{\w+\}").unwrap().find_iter(s).map(|m| m.as_str().to_string()).collect();
        v.sort();
        v
    }

    #[test]
    fn translation() {
        set_lang(Lang::Pl);
        assert_eq!(tr("Cancel"), "Anuluj");
        assert!(tr("⚠ an existing file — it will be replaced").contains("zastąpiony"));
        set_lang(Lang::En);
        assert_eq!(tr("Cancel"), "Cancel");
    }

    #[test]
    fn catalog_placeholders_match() {
        for (en, pl) in PL {
            assert_eq!(placeholders(en), placeholders(pl), "{en}");
        }
    }

    #[test]
    fn detection_order() {
        let get = |m: &'static [(&'static str, &'static str)]| {
            move |k: &str| m.iter().find(|(n, _)| *n == k).map(|(_, v)| v.to_string())
        };
        assert_eq!(detect_lang(get(&[])), Lang::En);
        assert_eq!(detect_lang(get(&[("LANG", "pl_PL.UTF-8")])), Lang::Pl);
        assert_eq!(detect_lang(get(&[("LANG", "pl_PL.UTF-8"), ("ROOTWHY_LANG", "en")])), Lang::En);
        assert_eq!(detect_lang(get(&[("LANGUAGE", "pl:en"), ("LANG", "de")])), Lang::Pl);
        assert_eq!(detect_lang(get(&[("LANGUAGE", ""), ("LANG", "PL_pl")])), Lang::Pl);
    }
}
