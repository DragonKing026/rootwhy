// SPDX-License-Identifier: GPL-3.0-or-later
// Tests of the binary itself (no GTK, no sudo): the Cli cases of tests/test_rootwhy.py and the hook tests of
// tests/test_hook.py, run against `rootwhy` and `rootwhy hook`.
use std::io::Write;
use std::process::{Command, Output, Stdio};

fn rootwhy() -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_rootwhy"));
    c.env("ROOTWHY_LANG", "en").env_remove("ROOTWHY_MODE");
    c
}

fn code(o: &Output) -> i32 {
    o.status.code().unwrap_or(-1)
}

#[test]
fn reason_required() {
    let o = rootwhy().arg("true").output().unwrap();
    assert_eq!(code(&o), 2);
    assert!(String::from_utf8_lossy(&o.stderr).contains("reason (-r) is required, 10–300 characters"));
}

#[test]
fn explain_option_removed() {
    // 2.0: descriptions come only from the system
    let o = rootwhy().args(["-r", "a valid reason here", "-e", "true=x", "true"]).output().unwrap();
    assert_eq!(code(&o), 2);
    assert!(String::from_utf8_lossy(&o.stderr).contains("unrecognized arguments: -e"));
}

#[test]
fn unknown_command() {
    let o = rootwhy().env("DISPLAY", ":0").args(["-r", "a valid reason here", "no-such-command-xyz"]).output().unwrap();
    assert_eq!(code(&o), 127);
}

#[test]
fn no_command_and_no_display() {
    let o = rootwhy().env("DISPLAY", ":0").args(["-r", "a valid reason here"]).output().unwrap();
    assert_eq!((code(&o), String::from_utf8_lossy(&o.stderr).trim()), (2, "rootwhy: no command given"));
    let o = rootwhy()
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .args(["-r", "a valid reason here", "true"])
        .output()
        .unwrap();
    assert_eq!((code(&o), String::from_utf8_lossy(&o.stderr).trim()), (2, "rootwhy: no graphical session for the dialog"));
}

#[test]
fn polish_messages() {
    let o = rootwhy().env("ROOTWHY_LANG", "pl").arg("true").output().unwrap();
    assert!(String::from_utf8_lossy(&o.stderr).contains("powód (-r) jest wymagany, 10–300 znaków"));
}

#[test]
fn askpass_refuses_without_sudo_parent() {
    let o = rootwhy().env("ROOTWHY_MODE", "askpass").output().unwrap();
    assert_eq!(code(&o), 1);
    assert!(String::from_utf8_lossy(&o.stderr).contains("askpass mode is only for sudo -A"));
    assert!(o.stdout.is_empty());
}

#[test]
fn version_and_help() {
    let o = rootwhy().arg("--version").output().unwrap();
    assert_eq!((code(&o), String::from_utf8_lossy(&o.stdout).as_ref()), (0, "rootwhy 3.0.0\n"));
    let o = rootwhy().env("COLUMNS", "80").arg("--help").output().unwrap();
    assert_eq!(code(&o), 0);
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.starts_with("usage: rootwhy [-h] [-r REASON] [--log-dir LOG_DIR] [--preview] [--version]\n               ...\n"));
    assert!(text.contains("  -r, --reason REASON  why the command is needed, 10–300 characters (required)\n"));
}

fn hook(input: &str) -> (i32, String) {
    let mut c = rootwhy().arg("hook").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(input.as_bytes()).unwrap();
    let o = c.wait_with_output().unwrap();
    (code(&o), String::from_utf8_lossy(&o.stderr).into_owned())
}

fn hook_cmd(command: &str) -> i32 {
    hook(&serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}}).to_string()).0
}

#[test]
fn hook_blocks_command_words() {
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
        "cat <<EOF\nx\nEOF\nsudo true",
        "cat <<EOF\n EOF\nEOF\nsudo true",
        "echo 'unbalanced; sudo true",
    ] {
        assert_eq!(hook_cmd(cmd), 2, "{cmd}");
    }
}

#[test]
fn hook_allows_mentions() {
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
        assert_eq!(hook_cmd(cmd), 0, "{cmd}");
    }
}

#[test]
fn hook_bad_input_passes() {
    assert_eq!(hook("not json").0, 0);
    assert_eq!(hook("").0, 0);
}

#[test]
fn hook_message_matches_python() {
    let (c, err) = hook_cmd_full("sudo true");
    assert_eq!(c, 2);
    assert_eq!(
        err,
        "Blocked: sudo called directly. Use rootwhy instead — the user sees a dialog with your reason and the command, and types the password there:\n  rootwhy -r \"What and why, 1–2 sentences in the user's language.\" -- command args…\nTell the user first that a password dialog is about to appear. Exit code 130 = cancelled: do not retry without asking. (If this project defines its own wrapper around rootwhy, use that.)\n"
    );
}

fn hook_cmd_full(command: &str) -> (i32, String) {
    hook(&serde_json::json!({"tool_name": "Bash", "tool_input": {"command": command}}).to_string())
}

#[test]
fn hook_ignores_the_askpass_variable() {
    // the environment must not be able to switch the hook off by selecting another mode
    let mut c =
        rootwhy().env("ROOTWHY_MODE", "askpass").arg("hook").stdin(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    c.stdin.take().unwrap().write_all(br#"{"tool_input":{"command":"sudo id"}}"#).unwrap();
    assert_eq!(code(&c.wait_with_output().unwrap()), 2);
}
