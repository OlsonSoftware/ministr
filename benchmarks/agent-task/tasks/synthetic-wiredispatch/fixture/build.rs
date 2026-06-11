//! Generates the wire dispatch table.
//!
//! Operation codes are NOT declared anywhere in the source tree: they
//! are assigned here, at build time, from the order of entries in
//! `ops/registry.list` (line N => op code N). Clients and the protocol
//! conformance suite bake those codes in, which is why the registry
//! file carries the "order is wire-significant" warning.
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=ops/registry.list");
    let listing = fs::read_to_string("ops/registry.list").expect("ops/registry.list");
    let mut arms = String::new();
    let mut op: u16 = 0;
    for line in listing.lines() {
        let entry = line.trim();
        if entry.is_empty() || entry.starts_with('#') {
            continue;
        }
        op += 1;
        arms.push_str(&format!(
            "        {op}u16 => Some(crate::handlers::{entry}(payload)),\n"
        ));
    }
    let gen = format!(
        "pub(crate) fn dispatch_op(op: u16, payload: &[u8]) -> Option<crate::codec::Reply> {{\n    match op {{\n{arms}        _ => None,\n    }}\n}}\n"
    );
    let out = Path::new(&env::var("OUT_DIR").unwrap()).join("dispatch_gen.rs");
    fs::write(out, gen).unwrap();
}
