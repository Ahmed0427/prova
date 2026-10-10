fn main() {
    sp1_build::build_program("../guest");
    println!("cargo:rerun-if-changed=../build/contract.o");
}
