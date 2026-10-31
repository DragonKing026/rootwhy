#![no_main]
// PreToolUse JSON from outside -> JSON parse, heredoc stripping, shell word splitting. Must never panic.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let _ = rootwhy_core::hook::decide(data);
    if let Ok(text) = std::str::from_utf8(data) {
        let _ = rootwhy_core::hook::shlex_split(text);
        let _ = rootwhy_core::hook::strip_heredocs(text);
    }
});
