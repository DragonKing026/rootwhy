// SPDX-License-Identifier: GPL-3.0-or-later
// rootwhy — run a command with sudo after a password dialog that explains why and what it does.
// https://github.com/DragonKing026/rootwhy
use rootwhy_core::{hook, modes};
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = match std::env::args_os().skip(1).map(|a| a.into_string()).collect() {
        Ok(a) => a,
        Err(_) => {
            eprintln!("rootwhy: arguments must be valid UTF-8");
            return ExitCode::from(2);
        }
    };
    // the plugin hook: `rootwhy hook` (checked before the askpass variable, so the environment cannot switch it off)
    if args.first().map(String::as_str) == Some("hook") {
        return ExitCode::from(hook::run() as u8);
    }
    let code = if std::env::var("ROOTWHY_MODE").as_deref() == Ok("askpass") { modes::askpass_mode() } else { modes::main_mode(&args) };
    ExitCode::from((code & 0xff) as u8)
}
