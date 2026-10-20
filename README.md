# prova

Run Solidity contracts natively inside the SP1 zkVM, and prove them.

Solang (branch `sp1-target`) compiles a contract to RV64IM machine code. The
guest here links that object, gives it storage, and runs one transaction per
SP1 execution. No EVM interpreter is involved: SP1 executes, and can prove, the
contract's own instructions.

```
contracts/foo.sol ──solang──► build/contract.o ──link──► guest ──SP1──► public values (+ proof)
```

## Requirements

- The SP1 toolchain (`sp1up`), which puts `cargo prove` and the `succinct`
  toolchain in `~/.sp1`.
- A Solang checkout on the `sp1-target` branch, at `~/void/projects/solang`
  (or set `SOLANG_DIR`).
- Solang's LLVM 16 build, at `~/llvm16/llvm16.0` (or set `LLVM_SYS_160_PREFIX`).
- A system LLVM with the RISC-V backend (`llc`, `llvm-objdump`). Solang's own
  LLVM can't emit RISC-V code, so it calls the system `llc`.

## Quick start

```bash
./prova setup                                   # once, and after switching Solang branches
./prova compile contracts/incrementer.sol
./prova run 'deploy 5' 'inc(uint32) 5' 'get()'
```

The first `run` builds `sp1-sdk` and takes a while. After that it takes a few
seconds. Output for one step:

```
== inc(uint32) 5
  calldata        0xdd5d5211…05
  sload  0x0 -> 0x5
  sstore 0x0 <- 0xa
  return 0x
  cycles          …
  storage before  {0x0 = 0x5}
  reverted        false
  output          0x
  storage after   {0x0 = 0xa}
```

The `sload`/`sstore`/`return` lines are the contract calling into the guest.

## Commands

| Command | What it does |
|---|---|
| `./prova setup` | Rebuilds Solang's RISC-V stdlib bitcode and Solang itself. |
| `./prova compile FILE.sol [NAME]` | Compiles with `--target riscv` and copies the object to `build/contract.o`. If the file has several contracts, pass `NAME`. |
| `./prova run STEP...` | Executes the steps in SP1, in order. Each step starts from the storage the previous one left. |
| `./prova prove STEP...` | Executes every step except the last, proves the last one, verifies the proof, then shows that a tampered copy is rejected. |
| `./prova test [FILE...]` | Runs test scripts, by default every `tests/*.test`. See [Tests](#tests). |
| `./prova inspect` | Prints the object's symbols and instruction set, and writes its disassembly to `build/contract.s`. |
| `./prova clean` | Removes `build/`. |

A step is one quoted transaction:

- `'deploy ARG...'` runs the constructor; the args are the constructor arguments.
- `'SIGNATURE ARG...'` calls a function, e.g. `'inc(uint32) 5'` or `'get()'`.
  The signature must be the canonical one (no spaces, no names), because its
  hash is the selector.

Each `ARG` is a single 32-byte ABI word: `42`, `-1`, `true`, `false` or
`0xHEX`. Hex is right-aligned like a number, so for `bytesN` arguments write
the full word: `bytes4 0xcafebabe` is
`0xcafebabe00000000000000000000000000000000000000000000000000000000`.

## Tests

`./prova test` runs every `tests/*.test` file. A test file names its
contract, then lists steps; the indented lines under a step say what its
result must be:

```
contract contracts/incrementer.sol

deploy 5
    storage 0x0 = 5
get()
    returns 5
nope()
    reverts
```

| Expectation | Meaning |
|---|---|
| `reverts` | the step reverts (without it, the step must succeed) |
| `returns WORD...` | the output is exactly these 32-byte words (none: empty) |
| `output 0xHEX` | the output is exactly these bytes |
| `storage SLOT = VALUE` | after the step; `0` also matches a slot never written |

Steps share storage, as with `run`. `#` starts a comment. On a failure the
step's guest trace is printed and the command exits non-zero.

## How it works

`./prova compile` produces an object file with exactly this interface:

```
T solang_deploy, solang_call      the contract's entry points (ptr, len)
U __sys_sload, __sys_sstore       storage, a slot and a value as 4×u64 limbs each
U __sys_return, __sys_revert      end of the transaction
```

`guest/build.rs` passes `build/contract.o` to the linker, and
`guest/src/main.rs` defines the four `__sys_*` functions. They aren't SP1
syscalls; they're plain function calls into Rust, which keeps storage in a
`BTreeMap`. The only real SP1 syscalls are reading the input, `commit` and
`halt`.

One SP1 run is one transaction:

1. Read `(deploy, calldata, storage)` from the host and commit them.
2. Call `solang_deploy` or `solang_call`.
3. On `__sys_return`, commit `(false, output, storage)`. On `__sys_revert`,
   commit `(true, output, storage before)`. Then halt.

The committed bytes are the public values. A proof says "this guest, with this
contract linked in, produced these public values". Changing any of them makes
verification fail, and so does linking a different contract, because that
changes the verifying key.

What it does **not** prove yet: that the starting storage is real chain state.
The host supplies it, and the proof only makes it public.

## Layout

```
prova           the script
contracts/      example contracts
tests/          test scripts for ./prova test
guest/          the SP1 program; links build/contract.o
host/           runs the guest: execute, or prove and verify
build/          compiler output (git-ignored)
docs/log/       one entry per change: what, why, decisions, verification
docs/review.md  a longer walkthrough of the Solang side and the design
```

## Gotchas

- **Proving needs a lot of RAM.** It didn't finish on a 14 GB laptop. Use a
  bigger machine, a GPU (`.cuda()` in `host/src/main.rs`), or Succinct's
  prover network (`.network()`).
- **`./prova setup` after switching Solang branches.** The r55 branch builds
  `target/riscv` with other flags, and `make` won't notice the difference.
- **One contract at a time.** `build/contract.o` is linked into the guest, so
  compiling another contract rebuilds the guest.
- **What's not supported yet:** mappings, `keccak256`, `string`/`bytes`,
  dynamic arrays, events, `msg.sender`, external calls. Using them makes
  Solang panic with `unimplemented`.
