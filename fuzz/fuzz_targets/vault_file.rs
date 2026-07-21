#![no_main]

use libfuzzer_sys::fuzz_target;
use silo_core::load_vault;
use std::{fs, path::PathBuf};

fuzz_target!(|input: &[u8]| {
    let path = PathBuf::from("/tmp/silo-fuzz-vault");
    let _ = fs::write(&path, input);
    let _ = load_vault(&path, "fuzz-password");
});
