//! Links every contract in build/contracts.txt and generates the code table
//! (`CODES`) that maps a contract name to its two entry points.

use std::fmt::Write;
use std::path::Path;

fn main() {
    let build = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build");
    let list = build.join("contracts.txt");
    println!("cargo:rerun-if-changed={}", list.display());
    let names = std::fs::read_to_string(&list)
        .expect("build/contracts.txt is missing: run `./prova compile <file.sol>` first");

    let mut table = String::from("static CODES: &[Code] = &[\n");
    let mut externs = String::from("extern \"C\" {\n");
    for name in names.lines().filter(|name| !name.is_empty()) {
        let object = build.join(format!("contracts/{name}.o"));
        println!("cargo:rustc-link-arg={}", object.display());
        println!("cargo:rerun-if-changed={}", object.display());
        writeln!(externs, "    fn prova_deploy_{name}(input: *const u8, len: u32);").unwrap();
        writeln!(externs, "    fn prova_call_{name}(input: *const u8, len: u32);").unwrap();
        writeln!(
            table,
            "    Code {{ name: {name:?}, deploy: prova_deploy_{name}, call: prova_call_{name} }},"
        )
        .unwrap();
    }
    externs.push_str("}\n");
    table.push_str("];\n");

    let out = Path::new(&std::env::var("OUT_DIR").unwrap()).join("contracts.rs");
    std::fs::write(out, externs + &table).unwrap();
}
