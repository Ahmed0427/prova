fn main() {
    sp1_build::build_program("../guest");

    // The host needs the names of the contracts linked into the guest.
    let list = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../build/contracts.txt");
    println!("cargo:rerun-if-changed={}", list.display());
    println!("cargo:rerun-if-changed=../build/contracts");
    let names = std::fs::read_to_string(&list).unwrap_or_default();
    let names: Vec<_> = names.lines().filter(|name| !name.is_empty()).collect();
    println!("cargo:rustc-env=PROVA_CONTRACTS={}", names.join(","));
}
