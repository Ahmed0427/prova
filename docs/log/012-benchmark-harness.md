# 012: benchmark harness: the same steps natively and in revm

## 1. What and why

Phase 3 compares running a contract natively (this repo's guest) with the
usual approach: running solc's EVM bytecode in an interpreter inside the
zkVM. This adds a second SP1 program, `evm-guest/` (revm with SP1's patched
`sha3`, so keccak is the same precompile on both sides), and `./prova bench
FILE.test`, which runs a test script's steps on both guests, requires the
same public values, and reports cycles split into setup / execute / commit
with SP1's cycle tracker. The guest's per-syscall trace can now be turned
off (it dominated cycle counts). A compute-heavy contract
(`contracts/compute.sol`) and its test join the ERC-20 as workloads.

Because both guests commit the same `Public`, the harness is also a
differential test of Solang against solc. It found three things (012 (f)):
precompile addresses in a test, a missing balance in the harness, and an
ABI decoding deviation (fixed in 011).

## 2. Decision

**(a) How revm gets the code: in the ELF or in the input?**

| Option | For | Against |
|---|---|---|
| A. Bake solc's bytecode into the EVM guest at build time (like the native contracts) | symmetric with the native guest (code fixed by the verifying key) | the EVM guest would rebuild whenever contracts change; and an EVM's code normally lives in state |
| **B. Pass the bytecode in the input (`EvmInput { input, codes }`)** | the EVM guest is built once; realistic (code is state); both guests read the same `Input` | the code is not committed or hashed into the public values here, so the EVM proof doesn't bind the code (a real system would check code hashes against a state root) |

Chosen: **B**, for a benchmark. revm still hashes every account's code when
loading it (`AccountInfo::from_bytecode`), which is why revm runs ~9-12
keccak-f permutations per transaction with no keccak in the contract; that
cost lands in its *setup* region.

**(b) Deploying at a chosen address in revm.** A CREATE transaction picks
the address from the sender and nonce; the native guest deploys at
`--to`. The EVM guest puts `init code ++ constructor args` *as the
account's code* at `--to` and calls it with empty calldata; what it returns
is the runtime code (checked against `solc --bin-runtime`, which rules out
contracts with immutables). Constructor semantics (CODECOPY of the args,
`address(this)`, storage writes) are the same as CREATE.

**(c) Splitting the cost.** SP1's cycle tracker: each guest prints
`cycle-tracker-report-start/end: setup|execute|commit` around reading the
input and building the state, running the contract / `evm.transact`, and
building and committing the public values. This needs sp1-sdk's `profiling`
feature in the host (it only adds the tracker; no profile is written unless
asked for). The markers' own printing costs a few hundred cycles each and
~10k cycles per run fall outside all three regions (entry point, allocator
set-up, halt).

**(d) The trace.** `Input::trace` (not committed): `run`/`test` keep it on,
`bench` turns it off. A `trace!` macro replaces the guest's `println!`s.

**(e) solc settings and the CSE workaround.** `solc 0.8.36 --optimize`
(200 runs, no via-IR). On the Solang side, `contracts/compute.sol`'s `sort`
miscompiles with Solang's own CSE pass (on every target, see (f)), so its
test file asks for `--no-cse` (a new `solang FLAGS` line, and
`SOLANG_FLAGS` for `./prova compile`). LLVM's -O2 does the same job: the
ERC-20 compiled with and without `--no-cse` differs by at most 2 cycles per
step (only on revert paths).

**(f) What the differential run found.**

1. *Addresses 0x0a and 0x0b are precompiles* (KZG point evaluation,
   BLS12-381 G1ADD) on Ethereum; revm ran those instead of the test's
   contracts. The native runtime has no precompiles. `tests/calls.test` now
   uses 0xaaaa/0xbbbb; documented in CLAUDE.md.
2. *Value without a balance.* revm halts with `OutOfFunds` when `msg.value`
   exceeds the sender's balance; the native runtime has no balances. The
   EVM guest now gives the sender a large balance, so the two agree.
3. *Oversized dynamic lengths* revert with `Panic(0x41)` under solc:
   fixed in Solang, log 011.
4. Known and expected: `mappings.test` disagrees on *storage* for a struct
   of two 64-bit fields (solc packs them in one slot; this target uses one
   slot per value, log 004 (c)); the outputs agree. `storage_types.sol`
   doesn't compile with solc (`delete` on a user-defined value type, which
   Solang accepts).
5. Solang bugs on every target, outside this branch's scope, found while
   writing `compute.sol`: (i) `(a[j], a[j+1]) = (a[j+1], a[j])` on a memory
   array panics the compiler ("casting basic value to slice"); (ii) CSE
   places a temp for an array length inside a loop body that does not
   dominate its uses after the loop, so bounds checks see a wrong length
   (`sort(1)` reverted with `Panic(0x32)`). Minimal repro of (ii): a
   function with three loops over the same `uint256[] memory` (see
   `contracts/compute.sol` `sort`), compiled without `--no-cse`.

## 3. How it works

```
./prova bench FILE.test
    compile the file's contracts with Solang (+ its `solang` flags)
    solc --optimize --bin --bin-runtime -o build/evm <contracts>
    prova-host --bench FILE

host --bench:
    codes = for each linked contract: (build/evm/NAME.bin, NAME.bin-runtime)
    native_state = evm_state = {}
    for step in script:
        n = execute(guest ELF,     Input { ..., state: native_state, trace: false })
        e = execute(evm-guest ELF, EvmInput { Input { ..., state: evm_state }, codes })
        require normalized(n.public) == normalized(e.public)   # zero slots dropped
        row: execute/total cycles of each, ratios, keccak-f counts
        native_state, evm_state = n.state_after, e.state_after
    write build/bench/FILE.md

evm-guest:
    [setup]   read EvmInput; CacheDB with each account's runtime code + storage
              sender gets a large balance
              deploy: account code = init ++ args, calldata = ""
              Context::mainnet() (no nonce/balance/base-fee/gas-cap checks), TxEnv(caller, to, value, data)
    [execute] evm.transact(tx)
    [commit]  Success -> commit state to the db, read back storage per account, logs
              Revert/Halt -> reverted, state unchanged
              commit(Public { ... })                 # the same type as the native guest
```

## 4. Real code

prova, `evm-guest/src/main.rs` (new): the transaction.

```rust
    let mut evm = Context::mainnet()
        .with_db(db)
        .modify_cfg_chained(|cfg| {
            cfg.disable_nonce_check = true;
            cfg.disable_balance_check = true;
            cfg.disable_base_fee = true;
            cfg.disable_block_gas_limit = true;
            cfg.disable_fee_charge = true;
            cfg.tx_gas_limit_cap = Some(u64::MAX);
        })
        .modify_block_chained(|block| { block.number = …; block.timestamp = …; block.gas_limit = u64::MAX; })
        .build_mainnet();
    let tx = TxEnv::builder()
        .caller(Address::from(context.caller))
        .kind(TxKind::Call(address))
        .value(U256::from_be_bytes(context.value))
        .data(calldata)
        .gas_limit(1 << 48)
        .gas_price(0)
        .build_fill();
    println!("cycle-tracker-report-end: setup");

    println!("cycle-tracker-report-start: execute");
    let outcome = (!deploying_over_code).then(|| evm.transact(tx).expect("revm failed"));
    println!("cycle-tracker-report-end: execute");
```

prova, `evm-guest/src/main.rs`: deploying at the chosen address.

```rust
    let calldata = match &input.deploy {
        Some(name) if !exists => {
            let mut init = code(name).init.clone();
            init.extend_from_slice(&input.calldata);
            db.insert_account_info(address, AccountInfo::from_bytecode(Bytecode::new_raw(init.into())));
            Bytes::new()
        }
        _ => Bytes::from(input.calldata.clone()),
    };
```

prova, `evm-guest/Cargo.toml` (new): revm and the keccak patch.

```toml
revm = { version = "38.0.0", default-features = false, features = [
    "optional_balance_check", "optional_block_gas_limit", "optional_fee_charge", "optional_no_base_fee",
] }
[patch.crates-io]
sha3 = { git = "https://github.com/sp1-patches/RustCrypto-hashes", tag = "patch-sha3-0.11.0-sp1-6.0.0" }
```

prova, `guest/src/main.rs`: regions and the trace switch.

```diff
+macro_rules! trace {
+    ($($arg:tt)*) => {
+        if unsafe { TRACE } {
+            println!($($arg)*);
+        }
+    };
+}
 ...
 pub fn main() {
+    println!("cycle-tracker-report-start: setup");
     let input: Input = read();
+    unsafe { TRACE = input.trace };
 ...
+    println!("cycle-tracker-report-end: setup");
+    println!("cycle-tracker-report-start: execute");
     let (reverted, output) = match entry {
```

prova, `host/src/bench.rs` (new): the comparison.

```rust
        let n = native(prover, &case.step, &native_state);
        let e = evm(prover, &case.step, &evm_state, &codes);
        if normalized(&n.public) != normalized(&e.public) {
            same = false;
            println!("MISMATCH {}", case.step.label);
            ...
        }
```

prova, `host/src/main.rs`: region cycles from the execution report.

```diff
+        regions: report.cycle_tracker.iter().map(|(k, v)| (k.clone(), *v)).collect(),
```

## 5. Verification

```
$ ./prova bench tests/compute.test     native and revm agree on every step
$ ./prova bench tests/erc20.test       native and revm agree on every step
$ ./prova bench tests/calls.test       native and revm agree on every step
$ ./prova bench tests/arith.test       agree
$ ./prova bench tests/events.test      agree
$ ./prova bench tests/context.test     agree   (after giving the sender a balance)
$ ./prova bench tests/mappings.test    DISAGREE on storage only: the packed struct (expected, 004)
$ ./prova bench tests/strings.test     DISAGREE on one step: the length band below 2^64 (011)
```

A step's breakdown (`tests/erc20.test`, `transfer`):

```
  --from 0xa11ce transfer(address,uint256) 0xb0b 300
      native   185355 =   13032 setup +    16008 execute +  150435 commit
      revm     410071 =  193396 setup +    57128 execute +  142792 commit
```

The ERC-20 with and without Solang's CSE: identical except 2 cycles on four
revert steps. With the trace on, `hashChain(100)` takes 10,543,173 cycles
(`./prova test`); with it off, 233,138.

```
$ ./prova test             all 11 test files passed (compute.test 10/10)
```

## 6. Commits

- Solang: none (the fix it found is 011).
- prova: `e8120a5 add a revm guest and ./prova bench to compare native and evm execution`

## 7. Open issues / follow-ups

- The EVM guest's code is input, not bound by the proof (a code hash check
  against a state root would be).
- Report the two Solang bugs in (f) 5 upstream; fix CSE's placement.
- The commit region dominates both guests (full state twice + logs):
  committing hashes instead would shrink it (log 006, 009).
