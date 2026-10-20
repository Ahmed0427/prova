# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

prova runs Solidity contracts natively inside the SP1 zkVM. Solang (a separate checkout, branch `sp1-target`) compiles a `.sol` file to an RV64IM object; the SP1 guest links that object directly and runs one transaction per SP1 execution. There is no EVM interpreter. `docs/review.md` is the long-form walkthrough of both the Solang changes and this repo's design.

## Commands

Everything goes through the `./prova` bash script, which sets `PATH` (`~/.sp1/bin`) and `LLVM_SYS_160_PREFIX`, and checks the Solang branch.

```bash
./prova setup                              # rebuild Solang + its RISC-V stdlib (once, and after switching Solang branches)
./prova compile contracts/incrementer.sol  # -> build/contracts/NAME.o for every contract, all linked into the guest
./prova run 'deploy 5' 'inc(uint32) 5' 'get()'   # execute steps in SP1, storage carried between steps
./prova prove 'deploy 5' 'inc(uint32) 5'   # execute all but last, prove+verify the last, check a tampered copy is rejected
./prova test                               # run every tests/*.test script (or name some)
./prova bench tests/erc20.test             # same steps natively and as solc bytecode in revm: compare results and cycles
./prova inspect                            # symbols, ISA, disassembly -> build/contracts/NAME.s
./prova clean                              # rm -rf build/
```

`run`/`prove` are `cargo run --release -q -p prova-host -- [--prove] STEP...`. A step is `'deploy [NAME] ARG...'` (NAME required when several contracts are linked; deploys at `--to`) or `'CANONICAL_SIG ARG...'` (no spaces in the signature, since its keccak is the selector). Each ARG is a 32-byte ABI word (`42`, `-1`, `true`, `false`, `0xHEX` right-aligned), a string `"text"` or bytes `hex"00ff"`; `host/src/abi.rs` tuple-encodes them (also for `returns` and `log ... data`).

Tests: `./prova test` runs the scripts in `tests/*.test`. Each names its contract (`contract FILE.sol [NAME]`), which the script compiles; then `prova-host --test FILE` runs the unindented lines as steps (sharing storage, like `run`) and checks the indented expectations under each: `reverts`, `error "msg"` (Error(string)), `returns VALUE...`, `output 0xHEX`, `storage [ADDRESS] SLOT = VALUE`, `syscall NAME COUNT` (precompile counts), `log [SIG] TOPIC... [data WORD...]`, `logs N`. A step without `reverts` must succeed. A line of only options sets defaults for later steps; several `contract FILE.sol` lines link several contracts. Each file rebuilds the guest (new objects), so a file takes ~30 s. The Solang-side tests are `cargo test --test riscv` in the Solang checkout; they need `LLVM_SYS_160_PREFIX` set. Run both before every commit.

Change log: every logical change gets `docs/log/NNN-name.md` (sections: what and why, decision, how it works, real code, verification, commits, open issues) and a line in `docs/log/README.md`. `docs/spec.md` is the contract/runtime interface (every `__sys_*` import, word layout, input and public values, each tagged implemented/planned); keep it current with every change to the interface. External requirements: SP1 toolchain (`sp1up`), Solang at `$SOLANG_DIR` (default `~/void/projects/solang`), Solang's LLVM 16 (`LLVM_SYS_160_PREFIX`, default `~/llvm16/llvm16.0`), and a system LLVM with the RISC-V backend (`llc`, `llvm-objdump`) — Solang's LLVM can't emit RISC-V, so it shells out to the system `llc`.

## Architecture

```
contracts/*.sol --solang--> build/contracts/*.o --link--> guest --SP1--> public values (+ proof)
```

- **Benchmark:** `evm-guest/` is a second SP1 program (revm 38, `default-features = false` + the `optional_*` cfg features, SP1's `sha3` patch for the keccak precompile). Its input is `prova_common::EvmInput` (the same `Input` + solc bytecode per contract name, so the ELF doesn't depend on the contracts) and it commits the same `Public`. `./prova bench` compiles with Solang and `solc --optimize --bin --bin-runtime` (→ `build/evm/`), then `prova-host --bench FILE` (`host/src/bench.rs`) runs each step on both with `trace: false`, requires equal public values (ignoring zero-valued slots), and prints cycles split by the guests' `cycle-tracker-report-*` regions setup/execute/commit (needs sp1-sdk's `profiling` feature, enabled in `host/Cargo.toml`). A deploy runs init code ++ args *as the account's code* at `--to`, so addresses match the native run.
- **Workspace layout:** the root workspace contains `host` and `common`; `evm-guest` is excluded like `guest`; `guest` is `exclude`d and is its own workspace (built for the `succinct` toolchain). `host/build.rs` calls `sp1_build::build_program("../guest")` and the host embeds the ELF via `include_elf!("prova-guest")`.
- **Linking the contracts:** `./prova compile` renames each object's `solang_deploy/solang_call` to `prova_deploy_NAME/prova_call_NAME` (`llvm-objcopy`) into `build/contracts/`, listing names in `build/contracts.txt`. `guest/build.rs` links them all and generates `CODES` (name → entry points) into `OUT_DIR/contracts.rs`; `host/build.rs` exposes the names as `env!("PROVA_CONTRACTS")`. Changing the set rebuilds the guest (new verifying key).
- **Contract ABI with the guest** (the object must have exactly this interface):
  - exports `solang_deploy(ptr, len)` and `solang_call(ptr, len)` (renamed per contract at link time); the shift helpers `__ashlti3` etc. are weak;
  - imports `__sys_sload`, `__sys_sstore`, `__sys_keccak256`, `__sys_return`, `__sys_revert` (see `docs/spec.md` for the full list), defined in `guest/src/main.rs`. These are ordinary C-ABI function calls into Rust, **not** SP1 syscalls. 256-bit slots/values are passed as four `u64` limbs, least significant first (`to_word`/`to_limbs`).
  - `__sys_return`/`__sys_revert` never return to the contract: they end the current call frame. The guest's `prova_enter`/`prova_leave` (`global_asm!` in `guest/src/main.rs`, a setjmp/longjmp over ra/sp/s0-s11) bring control back to the `run()` that entered the frame; frames skipped that way must own nothing that needs `Drop`. Storage writes go through a journal that a reverted frame rolls back; logs are truncated likewise. `main` commits after the outermost frame ends.
  - `__sys_call`/`__sys_staticcall`/`__sys_delegatecall` + `__sys_returndata_size/copy`: calls between linked contracts (0 = returned, 1 = reverted). Static frames reject `sstore`/`log`.
- **Transaction flow in the guest:** read one `prova_common::Input` (context, deploy: Option<name>, calldata, state: accounts of address/code name/storage) → deploy at or call `context.address` → at `__sys_return`/`__sys_revert` commit one `prova_common::Public` (inputs + reverted, output, logs, state_after; on revert no logs and state_after = state_before). Storage is one `BTreeMap<(Address, Word), Word>`.
- **Shared types:** `common/` (`prova-common`, `no_std` + serde) defines `Input`, `Context`, `Public`; both guest and host depend on it by path, so the input/public-values format has one definition. The host's `prove` path forges a copy by flipping `reverted` to demonstrate verification rejects tampering.
- **Context:** `--from/--value/--to/--block/--timestamp` before the steps set defaults; at the start of a step string they apply to that step (also in test files). Unauthenticated: only made public.
- **Host:** `host/src/script.rs` parses and checks test scripts. The host parses steps, computes selectors with `tiny-keccak`, runs each step via `CpuProver::execute` starting from the previous step's `state_after`, and prints guest stdout (the `sload`/`sstore`/`return` trace lines come from the guest's `println!`). Proving uses `.core()` on the CPU prover; swap to `.cuda()`/`.network()` in `host/src/main.rs` if needed.

## Gotchas

- The guest's per-syscall trace (`println!`) dominates cycle counts in `run`/`test` (e.g. 10.5M vs 233k for `hashChain(100)`); benchmarks set `Input::trace = false`.
- Addresses 0x01–0x11 (and 0x100) are precompiles on Ethereum: revm runs those instead of a contract deployed there. Use other addresses in tests that `./prova bench` runs.
- Solang's CSE pass (all targets) can misplace an array length used after a loop (`contracts/compute.sol` `sort`); `tests/compute.test` sets `solang --no-cse`.
- Proving needs a lot of RAM (did not finish on 14 GB). Execution (`run`) is cheap after the first `sp1-sdk` build.
- Re-run `./prova setup` after switching Solang branches: the r55 branch builds `target/riscv` with different flags and `make` won't notice. Also after editing `stdlib/*.c` in Solang: Solang's `build.rs` has no `rerun-if-changed` for the stdlib, so `cargo build` keeps the old bitcode (`make -C stdlib` fixes it).
- `__init_heap` is idempotent on riscv (re-entrant calls share a contract's heap); memory a frame allocated is not freed when it ends.
- The guest's `tiny-keccak` is patched to SP1's fork (`guest/Cargo.toml`), which runs the keccak precompile; `tests/mappings.test` checks `KECCAK_PERMUTE` counts. Both guests also patch `sha2` (sp1-zkvm hashes the public values with it), so `run` shows `SHA_EXTEND`/`SHA_COMPRESS` precompile calls.
- Unsupported Solidity features (Solang panics with `unimplemented`): storage arrays and strings, dynamic arrays in the ABI, `abi.encodePacked`, `new C()`, indexed event fields of reference types.
- Known deviations from solc: `type(intN).min / -1` reverts even in `unchecked`. RISC-V never traps on arithmetic, so every check wasm gets "for free" from a trap must be emitted explicitly (see log 003).
- Starting state is host-supplied and only made public, not proven to be real chain state.
