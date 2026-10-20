# prova

Run Solidity contracts natively inside the SP1 zkVM, and prove them.

Solang (branch `sp1-target`) compiles a contract to RV64IM machine code. The
guest here links that object, gives it storage, and runs one transaction per
SP1 execution. No EVM interpreter is involved: SP1 executes, and can prove, the
contract's own instructions.

```
contracts/foo.sol ──solang──► build/contracts/foo.o ──link──► guest ──SP1──► public values (+ proof)
```

## Requirements

- The SP1 toolchain (`sp1up`), which puts `cargo prove` and the `succinct`
  toolchain in `~/.sp1`.
- A Solang checkout on the `sp1-target` branch, at `~/void/projects/solang`
  (or set `SOLANG_DIR`).
- Solang's LLVM 16 build, at `~/llvm16/llvm16.0` (or set `LLVM_SYS_160_PREFIX`).
- A system LLVM with the RISC-V backend (`llc`, `llvm-objdump`). Solang's own
  LLVM can't emit RISC-V code, so it calls the system `llc`.

## Example: an ERC-20

`contracts/erc20.sol` is a plain EIP-20 token (the same source compiles with
solc 0.8). `tests/erc20.test` deploys it, transfers, checks balances,
allowances, events and storage slots, and checks that a transfer with too
little balance reverts:

```bash
./prova compile contracts/erc20.sol
./prova run --from 0xa11ce 'deploy 1000' 'transfer(address,uint256) 0xb0b 300' \
    'balanceOf(address) 0xb0b' '--from 0xb0b transfer(address,uint256) 0xca7 301'
./prova test tests/erc20.test
```

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
  state before    0xc0de incrementer {0x0 = 0x5}
  reverted        false
  output          0x
  state after     0xc0de incrementer {0x0 = 0xa}
```

The `sload`/`sstore`/`return` lines are the contract calling into the guest.

## Commands

| Command | What it does |
|---|---|
| `./prova setup` | Rebuilds Solang's RISC-V stdlib bitcode and Solang itself. |
| `./prova compile FILE.sol...` | Compiles every contract in the files with `--target riscv` into `build/contracts/NAME.o`, renaming each one's entry points so that all of them link into the guest. |
| `./prova run STEP...` | Executes the steps in SP1, in order. Each step starts from the state the previous one left. |
| `./prova prove STEP...` | Executes every step except the last, proves the last one, verifies the proof, then shows that a tampered copy is rejected. |
| `./prova test [FILE...]` | Runs test scripts, by default every `tests/*.test`. See [Tests](#tests). |
| `./prova inspect` | Prints the object's symbols and instruction set, and writes its disassembly to `build/contract.s`. |
| `./prova clean` | Removes `build/`. |

A step is one quoted transaction, optionally starting with context options
(see [Transaction context](#transaction-context)):

- `'deploy [NAME] ARG...'` deploys contract `NAME` at the `--to` address and
  runs its constructor with the args. `NAME` can be left out when only one
  contract is linked.
- `'SIGNATURE ARG...'` calls a function, e.g. `'inc(uint32) 5'` or `'get()'`.
  The signature must be the canonical one (no spaces, no names), because its
  hash is the selector.

Each `ARG` is a 32-byte ABI word (`42`, `-1`, `true`, `false` or `0xHEX`),
a `string` (`"text"`) or a `bytes` (`hex"00ff"`); the args are ABI-encoded
as a tuple, so strings get an offset and a tail. Hex is right-aligned like a number, so for `bytesN` arguments write
the full word: `bytes4 0xcafebabe` is
`0xcafebabe00000000000000000000000000000000000000000000000000000000`.

## Calls between contracts

Contracts linked into the same guest can call each other (`other.f(x)`,
`try`/`catch`, `.call`, `.staticcall`, `.delegatecall`). Each call runs in
its own frame; a revert undoes that frame's storage writes and events and
returns failure to the caller, and only a revert in the outermost frame
ends the transaction. `tests/calls.test` shows it:

```bash
./prova compile contracts/calls.sol
./prova run '--to 0xb deploy callee' '--to 0xa deploy caller 0xb' \
    '--to 0xa tryRevert(uint256) 9'
```

## Transaction context

`msg.sender`, `msg.value`, `address(this)`, `block.number` and
`block.timestamp` come from the host:

| Option | Sets | Default |
|---|---|---|
| `--from ADDRESS` | `msg.sender` | `0x1` |
| `--value N` | `msg.value` | `0` |
| `--to ADDRESS` | `address(this)` | `0xc0de` |
| `--block N` | `block.number` | `1` |
| `--timestamp N` | `block.timestamp` | `1` |

Before the steps, an option is the default for every step:
`./prova run --from 0xa11ce deploy 'whoami()'`. Inside a step, it is for that
step only: `'--from 0xb0b transfer(address,uint256) 0xa11ce 5'`.

The context is **not authenticated**: there is no signature and no chain.
The proof makes it public, so a verifier sees which sender a result is for,
but nothing proves that sender sent it.

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
| `error "message"` | the step reverts with `Error(string)` of this message (`require(cond, "message")`) |
| `returns VALUE...` | the output is exactly the ABI encoding of these values (words, `"text"`, `hex"..."`; none: empty) |
| `output 0xHEX` | the output is exactly these bytes |
| `storage [ADDRESS] SLOT = VALUE` | after the step, in the contract at `ADDRESS` (default: the step's `--to`); `0` also matches a slot never written |
| `syscall NAME COUNT` | an SP1 precompile ran exactly COUNT times, e.g. `KECCAK_PERMUTE` |
| `log [SIG] TOPIC... [data WORD...]` | the next event; `SIG` like `Transfer(address,address,uint256)` stands for topic 0 (its keccak256). With any `log` line, the number of events must match too |
| `logs N` | exactly N events |

Steps share state, as with `run`. A line with only options (`--to 0xb`)
sets them for the steps after it. `#` starts a comment. A file may have
several `contract` lines; all of them are linked. On a failure the
step's guest trace is printed and the command exits non-zero.

## How it works

The exact interface (entry points, every `__sys_*` import, word layout,
revert semantics, input and public values) is in [docs/spec.md](docs/spec.md).

`./prova compile` produces an object file with exactly this interface:

```
T solang_deploy, solang_call      the contract's entry points (ptr, len)
U __sys_sload, __sys_sstore       storage, a slot and a value as 4×u64 limbs each
U __sys_keccak256                 keccak256, for hashing and mapping slots
U __sys_log                       events (topics + data)
U __sys_call, __sys_staticcall,   calls to other contracts, and their return data
  __sys_delegatecall, __sys_returndata_size, __sys_returndata_copy
U __sys_caller, __sys_callvalue,  the transaction context
  __sys_address, __sys_block_number, __sys_timestamp
U __sys_return, __sys_revert      end of the transaction
```

`guest/build.rs` passes every `build/contracts/NAME.o` to the linker and
generates a table from contract names to entry points, and
`guest/src/main.rs` defines the `__sys_*` functions. They aren't SP1
syscalls; they're plain function calls into Rust, which keeps every
contract's storage in a `BTreeMap` keyed by (address, slot). The only real SP1 syscalls are reading the input, `commit` and
`halt`.

One SP1 run is one transaction:

1. Read `(context, deploy, calldata, state)` from the host. The state is a
   list of accounts: address, contract name, storage.
2. Deploy the named contract at `context.address`, or call the contract
   there.
3. On `__sys_return` or `__sys_revert`, commit the inputs together with
   `(reverted, output, logs, state after)`; after a revert there are no
   logs and the state after is the state before. Then halt.

The committed bytes are the public values. A proof says "this guest, with this
contract linked in, produced these public values". Changing any of them makes
verification fail, and so does linking a different contract, because that
changes the verifying key.

What it does **not** prove yet: that the starting state is real chain state.
The host supplies it, and the proof only makes it public.

## Layout

```
prova           the script
contracts/      example contracts
tests/          test scripts for ./prova test
common/         the input and public values types, shared by guest and host
guest/          the SP1 program; links build/contracts/*.o
host/           runs the guest: execute, or prove and verify
build/          compiler output (git-ignored)
docs/spec.md    the interface between a contract object and its runtime
docs/log/       one entry per change: what, why, decisions, verification
docs/review.md  a longer walkthrough of the Solang side and the design
```

## Gotchas

- **Proving needs a lot of RAM.** It didn't finish on a 14 GB laptop. Use a
  bigger machine, a GPU (`.cuda()` in `host/src/main.rs`), or Succinct's
  prover network (`.network()`).
- **`./prova setup` after switching Solang branches.** The r55 branch builds
  `target/riscv` with other flags, and `make` won't notice the difference.
- **The contracts are part of the program.** Compiling a different set
  rebuilds the guest and changes the verifying key.
- **What's not supported yet:** storage arrays and storage strings,
  dynamic arrays in the ABI, `abi.encodePacked`, creating contracts from
  contracts (`new C()`), balances and value transfer. Using them makes
  Solang panic with `unimplemented`.
- **Known deviations from solc:** `min / -1` reverts even in `unchecked`.
  See `docs/spec.md`.
