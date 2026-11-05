# prova

Run Solidity contracts natively inside the SP1 zkVM, and prove them.

Solang (branch `sp1-target`) compiles a contract to RV64IM machine code. The guest here
links that object, gives it storage, and runs one transaction per SP1 execution. No EVM
interpreter is involved: SP1 executes, and can prove, the contract's own instructions.

```
contracts/foo.sol ──solang──► build/contracts/foo.o ──link──► guest ──SP1──► public values
                                                                             (+ proof)
```

![Architecture: Solang compiles the contract to RV64IM, the guest links it, SP1 runs
it](docs/architecture.svg)

Supported so far: storage, mappings, `keccak256` (on SP1's keccak precompile), events,
`msg.sender`/`msg.value`/`address(this)`/block number and timestamp, `string`/`bytes` in
the ABI, `Error(string)` reverts, and `call`/`staticcall`/`delegatecall` between contracts
linked into the same guest, with try/catch.

## Requirements

- The SP1 toolchain (`sp1up`): `cargo prove` and the `succinct` toolchain, in `~/.sp1`.
- A Solang checkout on the `sp1-target` branch of [Ahmed0427/solang][fork].
  - It goes at `~/void/projects/solang`, or set `SOLANG_DIR`.
  - `git clone -b sp1-target https://github.com/Ahmed0427/solang ~/void/projects/solang`
- Solang's LLVM 16 build, at `~/llvm16/llvm16.0` (or set `LLVM_SYS_160_PREFIX`).
- A system LLVM with the RISC-V backend (`llc`, `llvm-objdump`).
  - Solang's own LLVM can't emit RISC-V code, so it calls the system `llc`.

[fork]: https://github.com/Ahmed0427/solang/tree/sp1-target

## Example: an ERC-20

`contracts/erc20.sol` is a plain EIP-20 token (the same source compiles with solc 0.8).
`tests/erc20.test` deploys it, transfers, checks balances, allowances, events and storage
slots, and checks that a transfer with too little balance reverts:

```bash
./prova compile contracts/erc20.sol
./prova run --from 0xa11ce 'deploy 1000' 'transfer(address,uint256) 0xb0b 300' \
    'balanceOf(address) 0xb0b' '--from 0xb0b transfer(address,uint256) 0xca7 301'
./prova test tests/erc20.test
```

## Quick start

```bash
./prova setup                         # once, and after switching Solang branches
./prova compile contracts/incrementer.sol
./prova run 'deploy 5' 'inc(uint32) 5' 'get()'
```

The first `run` builds `sp1-sdk` and takes a while. After that it takes a few seconds.
Output for one step:

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

The `sload`/`sstore`/`return` lines are the contract calling into the guest. That trace
costs most of the cycles shown; `./prova bench` turns it off.

## Commands

- `./prova setup`: rebuilds Solang's RISC-V stdlib bitcode and Solang itself.
- `./prova compile FILE.sol...`: compiles every contract in the files.
  - It uses `--target riscv` and writes `build/contracts/NAME.o`.
  - It renames each contract's entry points, so that all of them link into the guest.
- `./prova run STEP...`: executes the steps in SP1, in order.
  - Each step starts from the state the previous one left.
- `./prova prove STEP...`: executes every step except the last, and proves the last one.
  - It verifies the proof, then shows that a tampered copy is rejected.
  - The proof is saved to `build/proof.bin`.
- `./prova prove --network STEP...`: the same, proven by the Succinct prover network.
  - The proof is compressed and paid for in PROVE. It needs `NETWORK_PRIVATE_KEY`.
  - `SP1_PROVER=network` does the same as `--network`. Untested so far.
  - See [docs/network-proving.md](docs/network-proving.md).
- `./prova test [FILE...]`: runs test scripts, by default every `tests/*.test`.
  - See [Tests](#tests).
- `./prova bench FILE.test...`: runs a test script's steps two ways and compares cycles.
  - Natively, and as `solc --optimize` bytecode in revm (a second SP1 guest).
  - Both must give the same public values.
  - See [docs/benchmarks.md](docs/benchmarks.md).
- `./prova inspect`: prints the symbols and instruction set of the objects.
  - It writes the disassembly to `build/contracts/NAME.s`.
- `./prova clean`: removes `build/`.

`SOLANG_FLAGS` adds flags to `solang compile` (e.g. `SOLANG_FLAGS=--no-cse`); a test file
can also list them on a `solang FLAGS` line.

A step is one quoted transaction, optionally starting with context options (see
[Transaction context](#transaction-context)):

- `'deploy [NAME] ARG...'` deploys contract `NAME` at the `--to` address.
  - It runs the constructor with the args.
  - `NAME` can be left out when only one contract is linked.
- `'SIGNATURE ARG...'` calls a function, e.g. `'inc(uint32) 5'` or `'get()'`.
  - The signature must be the canonical one (no spaces, no names).
  - That is because its hash is the selector.

Each `ARG` is a 32-byte ABI word (`42`, `-1`, `true`, `false` or `0xHEX`), a `string`
(`"text"`) or a `bytes` (`hex"00ff"`). The args are ABI-encoded as a tuple, so strings get
an offset and a tail. Hex is right-aligned like a number, so for `bytesN` arguments write
the full word: `bytes4 0xcafebabe` is
`0xcafebabe00000000000000000000000000000000000000000000000000000000`.

## Benchmark

`./prova bench FILE.test` runs a test script's steps natively and as solc bytecode in revm
(both in SP1), checks they agree, and compares cycles. An ERC-20 `transfer` executes in
16k cycles natively against 57k in revm (139k against 364k with setup and commit).
Compute-heavy code is 3-63× faster. See [docs/benchmarks.md](docs/benchmarks.md).

## Calls between contracts

Contracts linked into the same guest can call each other (`other.f(x)`, `try`/`catch`,
`.call`, `.staticcall`, `.delegatecall`). Each call runs in its own frame; a revert undoes
that frame's storage writes and events and returns failure to the caller, and only a
revert in the outermost frame ends the transaction. `tests/calls.test` shows it:

```bash
./prova compile contracts/calls.sol
./prova run '--to 0xbbbb deploy callee' '--to 0xaaaa deploy caller 0xbbbb' \
    '--to 0xaaaa tryRevert(uint256) 9'
```

## Transaction context

`msg.sender`, `msg.value`, `address(this)`, `block.number` and `block.timestamp` come from
the host:

| Option | Sets | Default |
|---|---|---|
| `--from ADDRESS` | `msg.sender` | `0x1` |
| `--value N` | `msg.value` | `0` |
| `--to ADDRESS` | `address(this)` | `0xc0de` |
| `--block N` | `block.number` | `1` |
| `--timestamp N` | `block.timestamp` | `1` |

Before the steps, an option is the default for every step:
`./prova run --from 0xa11ce deploy 'whoami()'`. Inside a step, it is for that step only:
`'--from 0xb0b transfer(address,uint256) 0xa11ce 5'`.

The context is **not authenticated**: there is no signature and no chain. The proof makes
it public, so a verifier sees which sender a result is for, but nothing proves that
sender sent it.

## Tests

`./prova test` runs every `tests/*.test` file. A test file names its contract, then lists
steps; the indented lines under a step say what its result must be:

```
contract contracts/incrementer.sol

deploy 5
    storage 0x0 = 5
get()
    returns 5
nope()
    reverts
```

The expectations:

- `reverts`: the step reverts. Without it, the step must succeed.
- `error "message"`: the step reverts with `Error(string)` of this message.
  - This is what `require(cond, "message")` produces.
- `returns VALUE...`: the output is exactly the ABI encoding of these values.
  - Values are words, `"text"` or `hex"..."`. With no values, the output must be empty.
- `output 0xHEX`: the output is exactly these bytes.
- `storage [ADDRESS] SLOT = VALUE`: the slot's value after the step.
  - `ADDRESS` is the contract; the default is the step's `--to`.
  - `0` also matches a slot that was never written.
- `syscall NAME COUNT`: an SP1 precompile ran exactly COUNT times, e.g. `KECCAK_PERMUTE`.
- `log [SIG] TOPIC... [data WORD...]`: the next event.
  - `SIG`, like `Transfer(address,address,uint256)`, stands for topic 0 (its keccak256).
  - With any `log` line, the number of events must match too.
- `logs N`: exactly N events.

Steps share state, as with `run`. A line with only options (`--to 0xb`) sets them for the
steps after it. `#` starts a comment. A file may have several `contract` lines; all of
them are linked. On a failure the step's guest trace is printed and the command exits
non-zero.

The Solang side has its own tests: `cargo test --test riscv` in the Solang checkout, with
`LLVM_SYS_160_PREFIX` set. Run both before every commit.

## How it works

The exact interface (entry points, every `__sys_*` import, word layout, revert semantics,
input and public values) is in [docs/spec.md](docs/spec.md).

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

`guest/build.rs` passes every `build/contracts/NAME.o` to the linker and generates a table
from contract names to entry points, and `guest/src/main.rs` defines the `__sys_*`
functions. They aren't SP1 syscalls; they're plain function calls into Rust, which keeps
every contract's storage in a `BTreeMap` keyed by (address, slot). The only real SP1
syscalls are reading the input, `commit` and `halt`.

One SP1 run is one transaction:

1. Read `(context, deploy, calldata, state)` from the host.
   - The state is a list of accounts: address, contract name, storage.
2. Deploy the named contract at `context.address`, or call the contract there.
3. On `__sys_return` or `__sys_revert`, commit the inputs and the results, then halt.
   - The results are `(reverted, output, logs, state after)`.
   - After a revert there are no logs, and the state after is the state before.

### One transaction, step by step

`./prova run --from 0xa11ce 'deploy 1000' 'transfer(address,uint256) 0xb0b 300'` on the
ERC-20, second step:

1. **Compile.** `./prova compile contracts/erc20.sol` runs Solang with `--target riscv`.
   - It renames `solang_deploy`/`solang_call` to `prova_deploy_erc20`/`prova_call_erc20`.
   - It writes `build/contracts/erc20.o` and `build/contracts.txt`.
2. **Build.** `guest/build.rs` links every object in `contracts.txt`.
   - It generates the `CODES` table (name → entry points).
   - `host/build.rs` builds the guest ELF, and the host embeds it.
3. **Prepare.** The host builds the `Input` for the call.
   - The selector of `transfer(address,uint256)` is `a9059cbb`.
   - The args `0xb0b` and `300` are ABI-encoded.
   - The context is `from 0xa11ce`, `value 0`, `to 0xc0de`, and the block.
   - The state is the one the deploy step left.
4. **Execute.** The host runs the guest in SP1. Each step is its own SP1 run.
5. **Dispatch.** The guest finds the contract at `to` (`0xc0de` unless `--to` says so).
   - It opens a call frame and jumps to `prova_call_erc20`.
   - Solang's dispatcher matches `a9059cbb` to `transfer`.
6. **Run.** The contract calls the guest for everything outside itself:
   - `__sys_callvalue` (the non-payable check) and `__sys_caller` (`0xa11ce`);
   - `__sys_keccak256` (each balance's mapping slot);
   - `__sys_sload`, `__sys_sstore`, and `__sys_log` (the `Transfer` event).
7. **Return.** `__sys_return` restores the registers saved when the frame was entered.
   - So control comes back to the guest.
   - Had it been `__sys_revert`, the guest would roll back that frame's writes and events.
   - Only that frame's: the callers keep theirs.
8. **Commit.** The guest commits one `Public`.
   - Context, calldata, `state_before`, `reverted`, output, events and `state_after`.
9. **Repeat.** The host passes `state_after` into the next step.

The committed bytes are the public values. A proof says "this guest, with this contract
linked in, produced these public values". Changing any of them makes verification fail,
and so does linking a different contract, because that changes the verifying key.

What it does **not** prove yet: that the starting state is real chain state. The host
supplies it, and the proof only makes it public.

## Layout

```
prova           the script
contracts/      example contracts
tests/          test scripts for ./prova test
common/         the input and public values types, shared by guest and host
guest/          the SP1 program; links build/contracts/*.o
evm-guest/      the benchmark's baseline: the same transactions as solc bytecode in revm
host/           runs the guest: execute, or prove and verify
build/          compiler output (git-ignored)
docs/spec.md    the interface between a contract object and its runtime
docs/benchmarks.md  native vs revm cycles, and how to reproduce them
docs/evm-and-gas.md  design: calling legacy EVM contracts, gas metering
docs/review.md  a longer walkthrough of the Solang side and the design
docs/network-proving.md  proving on the Succinct prover network
```

## Gotchas

- **Proving needs a lot of memory and computing power.** There are three ways to do it:
  - a big machine;
  - a GPU (`.cuda()` in `host/src/main.rs`);
  - Succinct's prover network (`./prova prove --network`).
- **A network proof costs PROVE.** Start one only on purpose, smallest step first.
  - Executing (`run`) is cheap after the first `sp1-sdk` build.
- **Re-run `./prova setup` after switching Solang branches.**
  - The r55 branch builds `target/riscv` with other flags, and `make` won't notice.
- **After editing Solang's `stdlib/*.c`, run `make -C stdlib` in the Solang checkout.**
  - Solang's `build.rs` doesn't watch the stdlib, so `cargo build` keeps the old bitcode.
- **The contracts are part of the program.**
  - Compiling a different set rebuilds the guest and changes the verifying key.
- **The trace dominates cycle counts in `run` and `test`.**
  - E.g. 10.5M cycles against 233k for `hashChain(100)`.
  - Benchmarks turn it off (`Input::trace = false`).
- **`run` shows `SHA_EXTEND`/`SHA_COMPRESS` precompile calls.** They are expected.
  - SP1 hashes the public values with SHA-256 at the end of every run.
  - Both guests patch `sha2`, so that hashing uses SP1's SHA-256 precompiles.
- **Keccak also runs on a precompile.**
  - The guest patches `tiny-keccak` to SP1's fork (`guest/Cargo.toml`).
  - `tests/mappings.test` checks the `KECCAK_PERMUTE` counts.
- **Don't use addresses 0x01–0x11 (or 0x100) in tests that `./prova bench` runs.**
  - They are precompiles on Ethereum, so revm runs those instead of the contract.
- **Solang's CSE pass can misplace an array length used after a loop.**
  - This affects all targets; see `sort` in `contracts/compute.sol`.
  - `tests/compute.test` works around it with `solang --no-cse`.
- **`__init_heap` can run more than once per contract on riscv.**
  - Re-entrant calls share the contract's heap.
  - Memory a frame allocated is not freed when the frame ends.
- **What's not supported yet** (Solang panics with `unimplemented`):
  - storage arrays and storage strings;
  - dynamic arrays in the ABI;
  - `abi.encodePacked`;
  - creating contracts from contracts (`new C()`);
  - indexed event fields of reference types;
  - balances and value transfer.
- **Known deviations from solc:** `type(intN).min / -1` reverts even in `unchecked`.
  - RISC-V never traps on arithmetic, so every check must be emitted explicitly.
  - See `docs/spec.md`.
- **The starting state is not proven.** The host supplies it; the proof makes it public.
