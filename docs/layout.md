# Repository layout

What each folder and file in prova is for. Generated folders (`build/`, `target/`) are
listed at the end; they are not in git.

```
prova/
├── prova                  the command-line script: setup, compile, run, test, bench
├── Cargo.toml             workspace: host + common (the guests are separate)
├── common/                types shared by host and guests (Input, Public, State)
├── guest/                 SP1 program: the runtime that links and runs the contracts
├── evm-guest/             SP1 program: the same transactions run by revm (benchmark)
├── host/                  normal program: parses steps, runs SP1, checks tests
├── contracts/             example Solidity contracts
├── tests/                 test scripts, one per contract
└── docs/                  spec, benchmarks, design notes, overview
```

## Top level

| File | Purpose |
|---|---|
| `prova` | Bash entry point for every task. Sets `PATH` and `LLVM_SYS_160_PREFIX`, |
| | checks the Solang branch, then runs Solang, solc or cargo. |
| `Cargo.toml` | Workspace with `host` and `common`. `guest` and `evm-guest` are |
| | excluded: they build for SP1's RISC-V toolchain, not for the laptop. |
| `Cargo.lock` | Pinned versions for the workspace. |
| `README.md` | What prova is, requirements, commands, how one transaction runs. |
| `.gitignore` | Ignores `build/`, the `target/` folders, `*.o` and `*.s`. |

## `common/`: shared types

| File | Purpose |
|---|---|
| `Cargo.toml` | `no_std` crate with serde; used by the host and both guests. |
| `src/lib.rs` | One definition of the data that crosses the host/guest boundary: |
| | `Input` (context, deploy, calldata, state, trace), `Context`, `Account`, |
| | `State`, `Log`, `Public` (what the guest commits), and `EvmInput` for revm. |

## `guest/`: the native runtime (runs inside SP1)

| File | Purpose |
|---|---|
| `Cargo.toml` | Its own workspace for the `succinct` toolchain. Patches `tiny-keccak` |
| | and `sha2` to SP1's versions, so hashing uses the precompiles. |
| `Cargo.lock` | Pinned versions for the guest. |
| `build.rs` | Reads `build/contracts.txt`, links each `build/contracts/NAME.o`, and |
| | generates `CODES` (name -> deploy and call entry points) into `OUT_DIR`. |
| `src/main.rs` | The runtime. `main` reads `Input`, picks the entry point, runs it, |
| | commits `Public`. Also: call frames (`run`, `prova_enter`/`prova_leave`), |
| | the storage map and journal, and each `__sys_*` import the contract calls. |

## `evm-guest/`: the benchmark baseline (runs inside SP1)

| File | Purpose |
|---|---|
| `Cargo.toml` | revm 38, no default features, plus SP1's `sha3` and `sha2` patches. |
| `Cargo.lock` | Pinned versions for the revm guest. |
| `src/main.rs` | Reads `EvmInput` (the same `Input` plus solc bytecode per contract), |
| | runs it with revm, commits the same `Public` as the native guest. |

## `host/`: the program on your machine

| File | Purpose |
|---|---|
| `Cargo.toml` | Depends on `sp1-sdk` (`profiling` for cycle regions, `network` for |
| | `prove --network`) and `common`. |
| `build.rs` | Builds both guest ELFs with `sp1_build`, and passes the contract names |
| | from `build/contracts.txt` as `PROVA_CONTRACTS`. |
| `src/main.rs` | CLI: parses options and steps, computes selectors, builds `Input`, |
| | runs each step in SP1's executor, carries `state_after` to the next step, |
| | prints results. `--prove` proves and verifies the last step, locally or |
| | on the prover network (`SP1_PROVER=network`), and saves `build/proof.bin`. |
| `src/abi.rs` | Splits a step into tokens and ABI-encodes the arguments as a tuple |
| | (words in the head, strings and bytes in the tail). |
| `src/script.rs` | Parses `tests/*.test` files and checks each step's expectations |
| | (`reverts`, `returns`, `storage`, `log`, `syscall`, ...). |
| `src/bench.rs` | `--bench`: runs each test step on both guests, requires the same |
| | `Public`, prints cycles split into setup, execute and commit. |

## `contracts/`: example Solidity

| File | What it exercises |
|---|---|
| `incrementer.sol` | The first example: constructor, one write, one read. |
| `storage_types.sol` | Every value type in storage and in the ABI, enums, structs. |
| `arith.sol` | Division and modulo at many widths, by zero and on overflow. |
| `mappings.sol` | Mapping slots for many key types, nested mappings, structs. |
| `events.sol` | Plain, indexed, mixed, anonymous and empty events; events then revert. |
| `context.sol` | `msg.sender`, `msg.value`, `address(this)`, block number and time. |
| `strings.sol` | `string` and `bytes` in calldata and return data, bad offsets. |
| `erc20.sol` | A plain ERC-20: transfers, allowances, events, reverts. |
| `calls.sol` | Two contracts: `call`, `staticcall`, `delegatecall`, try/catch, reentry. |
| `compute.sol` | `fib`, `primes`, `sort`, `hashChain`: the compute benchmark. |

## `tests/`: test scripts

One `.test` file per contract, with the same name (plus `registry.test`). Each file
names its contract(s), then lists steps; the indented lines under a step are what must
hold after it. `./prova test` runs them all; `./prova bench` reuses them.

| File | What it checks |
|---|---|
| `incrementer.test` | Constructor, write, read, unknown selector. |
| `storage_types.test` | Storage words, ABI words, rejection of non-canonical input. |
| `arith.test` | `Panic(0x12)` on division by zero, signed overflow cases. |
| `mappings.test` | Slots equal solc's; keccak precompile call counts. |
| `events.test` | Topics and data equal solc's; no events after a revert. |
| `context.test` | Context options (`--from`, `--value`, ...) reach the contract. |
| `strings.test` | Offsets, lengths, padding, malformed calldata, `Panic(0x41)`. |
| `erc20.test` | 31 steps: balances, allowances, events, failing transfers. |
| `registry.test` | Two contracts from two files, each at its own address. |
| `calls.test` | Frames, return data, reverts undone per frame, static and delegate. |
| `compute.test` | Results checked against Python; compiled with `--no-cse`. |

## `docs/`

| File | Purpose |
|---|---|
| `prova-overview.md` | One-page overview: mission, goals, status, repositories. |
| `architecture.svg` | Diagram: compile, link, run in SP1, benchmark. |
| `spec.md` | The contract/runtime interface: entry points, every `__sys_*` import, |
| | word layout, `Input` and `Public`; each item implemented or planned. |
| `benchmarks.md` | Native vs revm cycle counts, how to reproduce, caveats. |
| `evm-and-gas.md` | Design only: calling legacy EVM contracts, and gas metering. |
| `network-proving.md` | Proving on the Succinct prover network: setup, costs, risks. |
| `review.md` | Walkthrough of the first version (one contract, four imports). |
| `layout.md` | This file. |

## Generated, not in git

| Path | Made by | Contents |
|---|---|---|
| `build/out/N/` | `./prova compile` | Solang's raw output for the Nth `.sol` file. |
| `build/contracts/NAME.o` | `./prova compile` | Objects, entry points renamed. |
| `build/contracts.txt` | `./prova compile` | Names of the contracts to link. |
| `build/contracts/NAME.s` | `./prova inspect` | Disassembly of each object. |
| `build/evm/` | `./prova bench` | solc: `NAME.bin` (deploy), `NAME.bin-runtime`. |
| `target/` | cargo | Host build output. |
| `guest/target/`, `evm-guest/target/` | `sp1_build` | Guest ELFs. |
