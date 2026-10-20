//! `--bench FILE`: runs a test script's steps natively and as solc bytecode
//! in revm, checks that both give the same public values, and compares their
//! cycles. Each guest marks three regions: setup (reading the input, building
//! the state), execute (the contract or the EVM transaction) and commit.

use crate::{contracts, fail, input, run_elf, script, Run, Step, ELF, EVM_ELF};
use prova_common::{EvmCode, EvmInput, Public, State};
use sp1_sdk::blocking::CpuProver;
use sp1_sdk::SP1Stdin;
use std::fmt::Write;
use std::path::Path;

/// The solc output for each linked contract, from build/evm (`./prova bench`
/// writes it with `solc --optimize --bin --bin-runtime`).
fn evm_codes() -> Vec<EvmCode> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build/evm");
    let read = |file: String| {
        let path = dir.join(&file);
        let text = std::fs::read_to_string(&path)
            .unwrap_or_else(|err| fail(&format!("{}: {err}", path.display())));
        let text = text.trim();
        (0..text.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap())
            .collect()
    };
    contracts()
        .into_iter()
        .map(|name| EvmCode {
            name: name.to_string(),
            init: read(format!("{name}.bin")),
            runtime: read(format!("{name}.bin-runtime")),
        })
        .collect()
}

fn native(prover: &CpuProver, step: &Step, state: &State) -> Run {
    let mut stdin = SP1Stdin::new();
    stdin.write(&input(step, state, false));
    run_elf(prover, ELF, stdin)
}

fn evm(prover: &CpuProver, step: &Step, state: &State, codes: &[EvmCode]) -> Run {
    let mut stdin = SP1Stdin::new();
    stdin.write(&EvmInput {
        input: input(step, state, false),
        codes: codes.to_vec(),
    });
    run_elf(prover, EVM_ELF, stdin)
}

/// The public values with zero-valued slots dropped: the native guest keeps a
/// slot written with 0, revm doesn't; both mean "0".
fn normalized(public: &Public) -> Public {
    let mut public = public.clone();
    for state in [&mut public.state_before, &mut public.state_after] {
        for account in state.iter_mut() {
            account.storage.retain(|(_, value)| *value != [0; 32]);
        }
    }
    public
}

fn region(run: &Run, name: &str) -> u64 {
    run.regions.get(name).copied().unwrap_or(0)
}

fn keccaks(run: &Run) -> u64 {
    run.syscalls
        .iter()
        .find(|(name, _)| name == "KECCAK_PERMUTE")
        .map_or(0, |(_, count)| *count)
}

pub fn bench(prover: &CpuProver, path: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_else(|err| fail(&format!("{path}: {err}")));
    let cases = script::parse(&text).unwrap_or_else(|err| fail(&format!("{path}: {err}")));
    let codes = evm_codes();

    let mut table = String::new();
    writeln!(
        table,
        "| step | native execute | native total | revm execute | revm total | execute ×  | total × | keccak-f (native / revm) |"
    )
    .unwrap();
    writeln!(table, "|---|--:|--:|--:|--:|--:|--:|--:|").unwrap();

    let (mut native_state, mut evm_state) = (State::new(), State::new());
    let mut same = true;
    for case in &cases {
        let n = native(prover, &case.step, &native_state);
        let e = evm(prover, &case.step, &evm_state, &codes);
        if normalized(&n.public) != normalized(&e.public) {
            same = false;
            println!("MISMATCH {}", case.step.label);
            println!(
                "  native: reverted {} output 0x{}",
                n.public.reverted,
                crate::hex(&n.public.output)
            );
            println!(
                "  revm:   reverted {} output 0x{}",
                e.public.reverted,
                crate::hex(&e.public.output)
            );
            if n.public.logs != e.public.logs {
                println!("  the events differ");
            }
            if normalized(&n.public).state_after != normalized(&e.public).state_after {
                println!("  the state after differs");
            }
            for line in e
                .trace
                .lines()
                .filter(|line| !line.starts_with("cycle-tracker"))
            {
                println!("  revm: {line}");
            }
        }
        let (ne, ee) = (region(&n, "execute"), region(&e, "execute"));
        writeln!(
            table,
            "| `{}` | {} | {} | {} | {} | {:.1} | {:.1} | {} / {} |",
            case.step.label,
            ne,
            n.cycles,
            ee,
            e.cycles,
            ee as f64 / ne.max(1) as f64,
            e.cycles as f64 / n.cycles.max(1) as f64,
            keccaks(&n),
            keccaks(&e),
        )
        .unwrap();
        let regions = |run: &Run| {
            format!(
                "{:>8} = {:>7} setup + {:>8} execute + {:>7} commit",
                run.cycles,
                region(run, "setup"),
                region(run, "execute"),
                region(run, "commit")
            )
        };
        println!("  {}", crate::abbreviate(&case.step.label, 70));
        println!("      native {}", regions(&n));
        println!("      revm   {}", regions(&e));
        native_state = n.public.state_after;
        evm_state = e.public.state_after;
    }

    println!("\n{table}");
    let out = Path::new(env!("CARGO_MANIFEST_DIR")).join("../build/bench");
    std::fs::create_dir_all(&out).unwrap();
    let stem = Path::new(path).file_stem().unwrap().to_string_lossy();
    std::fs::write(out.join(format!("{stem}.md")), &table).unwrap();
    println!(
        "{path}: {} (table in build/bench/{stem}.md)",
        if same {
            "native and revm agree on every step"
        } else {
            "native and revm DISAGREE"
        }
    );
    same
}
