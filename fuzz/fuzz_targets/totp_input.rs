#![no_main]

use libfuzzer_sys::fuzz_target;
use silo_core::generate_totp;

fuzz_target!(|input: &[u8]| {
    let value = String::from_utf8_lossy(input);
    let _ = generate_totp(&value, 1_700_000_000);
});
