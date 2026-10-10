# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

prova runs Solidity contracts natively inside the SP1 zkVM. Solang (a separate checkout, branch `sp1-target`) compiles a `.sol` file to an RV64IM object; the SP1 guest links that object directly and runs one transaction per SP1 execution. There is no EVM interpreter. `docs/review.md` is the long-form walkthrough of both the Solang changes and this repo's design.

## Commands

Everything goes through the `./prova` bash script, which sets `PATH` (`~/.sp1/bin`) and `LLVM_SYS_160_PREFIX`, and checks the Solang branch.

```bash
./prova setup                              # rebuild Solang + its RISC-V stdlib (once, and after switching Solang branches)
./prova compile contracts/incrementer.sol  # -> build/contract.o (pass NAME if the file has several contracts)
./prova run 'deploy 5' 'inc(uint32) 5' 'get()'   # execute steps in SP1, storage carried between steps
./prova prove 'deploy 5' 'inc(uint32) 5'   # execute all but last, prove+verify the last, check a tampered copy is rejected
./prova inspect                            # symbols, ISA, disassembly -> build/contract.s
./prova clean                              # rm -rf build/
```

`run`/`prove` are `cargo run --release -q -p prova-host -- [--prove] STEP...`. A step is `'deploy ARG...'` or `'CANONICAL_SIG ARG...'` (no spaces in the signature, since its keccak is the selector). Each ARG is exactly one 32-byte ABI word (`42`, `-1`, `true`, `false`, `0xHEX` right-aligned).

There are no tests in this repo; the Solang-side tests live in the Solang checkout (`tests/riscv.rs`). External requirements: SP1 toolchain (`sp1up`), Solang at `$SOLANG_DIR` (default `~/void/projects/solang`), Solang's LLVM 16 (`LLVM_SYS_160_PREFIX`, default `~/llvm16/llvm16.0`), and a system LLVM with the RISC-V backend (`llc`, `llvm-objdump`) — Solang's LLVM can't emit RISC-V, so it shells out to the system `llc`.

## Architecture

```
contracts/*.sol --solang--> build/contract.o --link--> guest --SP1--> public values (+ proof)
```

- **Workspace layout:** the root workspace contains only `host`; `guest` is `exclude`d and is its own workspace (built for the `succinct` toolchain). `host/build.rs` calls `sp1_build::build_program("../guest")` and the host embeds the ELF via `include_elf!("prova-guest")`.
- **Linking the contract:** `guest/build.rs` passes `build/contract.o` as a raw linker arg. Both build scripts `rerun-if-changed` on it, so compiling a different contract rebuilds the guest (and changes the verifying key). Only one contract can be linked at a time.
- **Contract ABI with the guest** (the object must have exactly this interface):
  - exports `solang_deploy(ptr, len)` and `solang_call(ptr, len)`;
  - imports `__sys_sload`, `__sys_sstore`, `__sys_return`, `__sys_revert`, defined in `guest/src/main.rs`. These are ordinary C-ABI function calls into Rust, **not** SP1 syscalls. 256-bit slots/values are passed as four `u64` limbs, least significant first (`to_word`/`to_limbs`).
  - `__sys_return`/`__sys_revert` never return: they commit and call `syscall_halt`. Reaching the end of `main` is `unreachable!`.
- **Transaction flow in the guest:** read `(deploy: bool, calldata: Vec<u8>, storage: Vec<(Word, Word)>)` → commit those three → call the entry point → on return commit `(false, output, storage_after)`, on revert commit `(true, output, storage_before)`. Storage is a `static mut BTreeMap`.
- **Public values contract:** the order/types the guest commits must match `Public::read`/`Public::write` in `host/src/main.rs` exactly (6 fields). Change both together. The host's `prove` path forges a copy by flipping `reverted` to demonstrate verification rejects tampering.
- **Host:** parses steps, computes selectors with `tiny-keccak`, runs each step via `CpuProver::execute` starting from the previous step's `storage_after`, and prints guest stdout (the `sload`/`sstore`/`return` trace lines come from the guest's `println!`). Proving uses `.core()` on the CPU prover; swap to `.cuda()`/`.network()` in `host/src/main.rs` if needed.

## Gotchas

- Proving needs a lot of RAM (did not finish on 14 GB). Execution (`run`) is cheap after the first `sp1-sdk` build.
- Re-run `./prova setup` after switching Solang branches: the r55 branch builds `target/riscv` with different flags and `make` won't notice.
- Unsupported Solidity features (Solang panics with `unimplemented`): mappings, `keccak256`, `string`/`bytes`, dynamic arrays, events, `msg.sender`, external calls.
- Starting storage is host-supplied and only made public, not proven to be real chain state.
