// SPDX-License-Identifier: GPL-3.0-or-later
//! rootwhy — run a command as root after a password dialog that explains why and what the command does.
//! https://github.com/DragonKing026/rootwhy
//!
//! This crate is the part that must not depend on GTK: tokenizing, man pages, warnings, the two modes, the
//! hook. The window itself is the separate binary `rootwhy-gtk`.

pub mod cli;
pub mod command;
pub mod dialog;
pub mod explain;
pub mod hook;
pub mod i18n;
pub mod man;
pub mod modes;
pub mod util;
