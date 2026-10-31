#![no_main]
// Command line from outside -> option parser and tokenizer of the shown command. Must never panic.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    let args: Vec<String> = text.split('\0').map(str::to_owned).collect();
    let _ = rootwhy_core::cli::parse_args_cols(&args, 80);
    let _ = rootwhy_core::command::tokenize(&args, None);
});
