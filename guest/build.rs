use std::path::Path;

fn main() {
    let object = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build/contract.o");
    assert!(
        object.exists(),
        "build/contract.o is missing: run `./prova compile <file.sol>` first"
    );
    println!("cargo:rustc-link-arg={}", object.display());
    println!("cargo:rerun-if-changed={}", object.display());
}
