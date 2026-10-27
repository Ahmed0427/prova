# Solang → SP1: review notes

> This walkthrough describes the first version (one contract, four
> `__sys_*` imports). Everything since — keccak and mappings, events, the
> transaction context, string ABI, several contracts and calls between
> them, the revm benchmark — is described in `README.md`, and the current
> interface is `docs/spec.md`.

Branch `sp1-target` in `~/void/projects/solang`, plus this repository
(`~/void/projects/prova`): `guest/`, `host/` and the `./prova` script.

---

## 1. The one-paragraph pitch

We taught the Solidity compiler Solang a new target, `--target riscv`. It turns a
`.sol` contract into **native RISC-V machine code (RV64IM)**: a plain object
file, `contract.o`. A small Rust program (the **guest**) links that object and
runs inside **SP1**, a zkVM that proves RISC-V execution. The contract's
storage reads and writes, and its return and revert, go through four
functions the guest provides. The result is that a Solidity contract runs, and
can be proven, in SP1 **without an EVM interpreter**.

```
  incrementer.sol ──solang──► incrementer.o ──link──► guest ELF ──SP1──► result + proof
   (Solidity)                 (RV64IM code)          (Rust + contract)
```

---

## 2. Five words you must know

| Word | Meaning, simply |
|---|---|
| **zkVM (SP1)** | A RISC-V computer whose every step can be proven. You run a program and get a *proof* that it ran correctly. |
| **guest** | The program running *inside* SP1. Here: Rust code plus our contract. |
| **host** | The normal program on your laptop that feeds inputs to the guest and reads its outputs. It is **untrusted**. |
| **public values** | The bytes the guest `commit`s. A proof is a statement about exactly these bytes. |
| **syscall layer** | The four `__sys_*` functions. Contract code calls them for storage, return and revert. Swap them and the same contract runs somewhere else. |

---

## 3. Big picture: who calls whom

```
 ┌──────────────── HOST (host/, your laptop) ──────────────────────┐
 │  stdin: deploy?, calldata, storage                               │
 └───────────────┬──────────────────────────────────▲──────────────┘
                 │ inputs                            │ public values (+ proof)
 ┌───────────────▼────────── GUEST ELF in SP1 ───────┴──────────────┐
 │  guest/src/main.rs                 (Rust, written by us)          │
 │    main(): read → commit inputs → solang_call / solang_deploy     │
 │        │                               ▲                          │
 │        ▼                               │ callbacks                │
 │  ┌──────────────────────────────┐   __sys_sload / __sys_sstore   │
 │  │ contract.o  (made by Solang) │   __sys_return / __sys_revert  │
 │  │  solang_call  solang_deploy  │──►  (implemented in main.rs    │
 │  └──────────────────────────────┘      over a BTreeMap)          │
 └───────────────────────────────────────────────────────────────────┘
```

The key idea: **the contract never knows where it runs.** It only calls
`__sys_sload(slot)` and so on. On the r55 branch those same functions are
`ecall`s into r55. On SP1 they're Rust functions working on a map.

---

## 4. The Solang side, file by file

The compiler pipeline, and where our code plugs in:

```
 .sol ─► sema ─► codegen (CFG) ─► emit (LLVM IR) ─► external llc ─► .o
                     ▲                  ▲                ▲
         targets/riscv/*.rs     emit/riscv/*.rs   emit/riscv/codegen.rs
        (dispatch + ABI)      (storage, entry points)  (RV64IM machine code)
```

### 4.1 Plumbing: making `--target riscv` exist

Small edits so the compiler knows the target:

- `src/lib.rs:36` adds `Target::Riscv`; `:86` parses `"riscv"`; `:96` gives
  the output extension `.o`.
- `src/bin/cli/mod.rs` adds `riscv` to the allowed `--target` values.
- `src/sema/namespace.rs:39`: `(20, 32)` means a 20-byte address and 32-byte
  values, as on Ethereum.
- `src/sema/expression/literals.rs:136` allows Ethereum address literals
  (`0x...dEaD`).
- `src/emit/mod.rs:328-346`: LLVM target `riscv64`, triple
  `riscv64-unknown-none-elf`, features **`+m`**. That's RV64IM: no compressed
  (`c`) and no atomic (`a`) instructions, because SP1 doesn't support them.

### 4.2 `src/codegen/targets/riscv/dispatch.rs`: "which function runs?"

This builds two small router functions, in Solang's own IR (the CFG), not in
LLVM IR yet:

```
 riscv_call_dispatch(calldata):          riscv_deploy_dispatch(args):
   len < 4 ?  → revert                     decode constructor args
   selector = first 4 bytes                call constructor
   switch selector:                        Return (back to the caller)
     0xdd5d5211 → inc(uint32)
     0x6d4ce63c → get()
     else       → revert
   decode args → call fn → encode result → ReturnData
```

- `:193 build_call` builds the call router; `:266 Instr::Switch` is the
  `switch`; `:273 AssertFailure` is the revert for an unknown selector.
- `:250 from_bytes_le`: Solang reads the 4 selector bytes as a little-endian
  number, so the constants are byte-reversed to match.
- `:320 dispatch_case`: decode the arguments, call the function, ABI-encode
  the result, `ReturnData`. `ReturnData` becomes a call to `__sys_return`.
- `:169 build_deploy` ends with `Instr::Return` (`:188`), a normal function
  return, so control goes back to `solang_deploy`, which then calls
  `__sys_return` itself.

### 4.3 `src/codegen/targets/riscv/encoding.rs`: the Ethereum ABI

Calldata and return data use the **Ethereum ABI**: every value is one
32-byte big-endian word, and structs and fixed arrays are laid out inline.

```
 inc(5) calldata:  dd5d5211 | 00000000…00000005
                   selector   uint32 5, as one 32-byte word
```

- `:29 static_size(ty)`: every supported type has a size known at compile
  time: 32 bytes per value, `n × element` for `T[n]`, the sum of the fields
  for a struct.
- `:93 widen`: turns a value into its 32-byte word. `intN` is sign-extended,
  `bytesN` is shifted to the left (left-aligned), and an `address` becomes a
  number.
- `:189 read_value_type`: reads a word **and checks it's canonical**. It
  computes `widen(decoded) == word`, or `word < number of variants` for an
  enum. A `bool` of 2, or a `uint8` with high bits set, reverts, just like
  solc.
- `:320 encode` and `:419 read_from_buffer`: the per-type cases. Structs go
  field by field; fixed arrays go through a loop over elements.
- **Why not reuse Solang's shared encoder?** It has a `memcpy` shortcut that
  copies Solang's in-memory layout (little-endian, packed). That's fine for
  Polkadot's SCALE encoding, and wrong for the Ethereum ABI.
- `src/codegen/targets/buffer_validator.rs:255`: the up-front calldata length
  check uses `static_size` (32 bytes per value) instead of in-memory sizes.
  Before this fix, calldata that was too short was accepted.

### 4.4 `src/emit/riscv/target.rs`: storage as 256-bit words

Storage is like Ethereum's: a map from a 256-bit slot to a 256-bit word.
Every value takes one slot; structs and arrays take one slot per value inside
them.

```
 Solidity value  ─to_slot_word─►  256-bit word ─split_256─► 4×u64 ─► __sys_sstore
 Solidity value  ◄from_slot_word─  256-bit word ◄─ 4×u64  ◄──────── __sys_sload
```

- `:29 sload` and `:53 sstore` call the guest. A 256-bit number doesn't fit
  in one register, so it's passed as **4 × u64 limbs, least significant
  first** (`:66 split_256`).
- `:82 to_slot_word` and `:131 from_slot_word`:
  - `bool` is stored as 0 or 1;
  - integers are zero-extended;
  - `address` is the interesting case. Solang keeps it as 20 big-endian
    bytes, so storing it means byte-swapping it into a number
    (`llvm.bswap.i160`).
- `:262 load_slots`, `:307 store_slots` and `:347 delete_slots` go through
  structs and fixed arrays recursively, one slot per value.
- `:384 declare_syscalls` declares the `__sys_*` functions. They're
  **declared, not defined**, so the linker expects the guest to provide them.

### 4.5 `src/emit/riscv/mod.rs`: the two entry points

The emitted object exports only two functions:

```
 solang_deploy(ptr, len)              solang_call(ptr, len)
   __init_heap()                        __init_heap()
   storage_initializer()   ← uint x=5;  riscv_call_dispatch(ptr, len)
   riscv_deploy_dispatch(ptr, len)        (ends in __sys_return/__sys_revert)
   __sys_return(null, 0)
```

- `:89 emit_deploy`: state variable initializers run **before** the
  constructor, as in Solidity.
- `:61 internalize`: hides every other symbol (`__malloc`, `__memcpy`, …) so
  they can't clash with the guest's own symbols, which shrank the object from
  69 KB to 10 KB.
  - Three shift helpers (`__ashlti3`, …) stay visible because LLVM calls them
    by name.
  - We wrote our own version because Solang's `Binary::internalize` has a
    bug: it uses `.all(...)`, so with two export names it never hides
    anything.

### 4.6 `src/emit/riscv/codegen.rs` and `src/emit/binary.rs`: making machine code

- Solang's own LLVM 16 build has **no RISC-V backend**, so we write the
  bitcode to a file and run an external `llc` (`:16 object_from_module`)
  with `-mtriple=riscv64-unknown-none-elf -mattr=+m -code-model=medium`.
  Those settings match SP1's `riscv64im-succinct-zkvm-elf` target exactly.
- `:60 find_llc` searches **all of PATH**, because the custom LLVM's `llc`
  comes first and lacks RISC-V.
- `src/emit/binary.rs:225`: the riscv target returns the object **without
  linking**. Linking happens in the guest's build.

### 4.7 The stdlib (C runtime helpers Solang links into every contract)

- `stdlib/Makefile:25` builds them with `--target=riscv64 -march=rv64im
  -mcmodel=medany` into `target/riscv/`.
- `stdlib/heap.c:12-15`: Solang's heap is a **64 KB static array**, because
  the guest's Rust allocator owns the rest of memory. When it runs out, it
  calls `__sys_revert`.

### 4.8 Tests: `tests/riscv.rs`

- `object_is_rv64im`: a 64-bit RISC-V relocatable object, `e_flags == 0` (no
  compressed instructions), and an arch string starting `rv64i2p1_m2p0` with
  no `_a` or `_c`.
- `object_exports_entry_points_and_imports_syscalls`: it defines exactly
  `solang_deploy`, `solang_call` and the 3 shift helpers, and leaves exactly
  the four `__sys_*` undefined.

---

## 5. The SP1 side (`~/void/projects/prova`)

### 5.1 `guest/build.rs`: linking the contract

```rust
println!("cargo:rustc-link-arg={}", object.display()); // object = build/contract.o
```
Cargo passes `build/contract.o` to the linker, so `solang_call` and the other
symbols resolve. `./prova compile` puts the object there. **One contract per
build.**

### 5.2 `guest/src/main.rs`: the guest

| Lines | What it does |
|---|---|
| 13-16 | Declares the two functions that `contract.o` exports. |
| 19-25 | Reads `deploy`, `calldata` and `storage` from the host and **commits them**, so the proof says *which* call on *which* state. |
| 28-32 | Loads storage into a `BTreeMap`, keeps a copy in `BEFORE`, and calls into the contract. |
| 39-49 | `to_word` / `to_limbs`: 4 × u64 limbs (least significant first) ↔ 32 big-endian bytes. |
| 52-66 | `__sys_sload` / `__sys_sstore`: read and write the map; `println!` makes each call visible. |
| 69-78 | `__sys_return` commits the map; `__sys_revert` commits `BEFORE`, because a revert undoes all writes. |
| 88-92 | Commits `reverted`, the return data and the storage after, then `syscall_halt(0)`. This seals the public values and stops. |

`syscall_halt` never returns, which is why `__sys_return` has the type
`-> !` and `main` ends in `unreachable!`.

### 5.3 `host/src/main.rs`: the host

- `:7 include_elf!("prova-guest")`: the guest ELF, built by `host/build.rs`
  through `sp1_build`.
- `:74 parse_step` and `:98 parse_word`: each command-line step, like
  `'inc(uint32) 5'`, becomes calldata (a 4-byte selector, then 32-byte words).
- `:157 execute`: runs the guest in **SP1's executor**, which gives exact
  results and a cycle count but **no proof**. `.stdout(...)` captures the
  guest's syscall lines.
- `:222 main`: runs the steps in order, passing each step's storage after on
  to the next.
- `:176 prove` (`./prova prove`):
  1. `setup` gives the verifying key;
  2. `prove(...).core()` generates the proof;
  3. `verify` checks it;
  4. it then flips the `reverted` flag in the public values and expects
     verification to fail.

Proving needs more RAM than this 14 GB laptop has. Setup worked and printed
a verifying key, but the proof never finished.

---

## 6. One transaction end to end: `inc(5)`, storage `{slot0 = 5}`

```
host   stdin = (false, dd5d5211|..05, {0:5})
guest  commit inputs
guest  solang_call(calldata)                       ─┐
 .o      __init_heap                                │
 .o      riscv_call_dispatch: selector dd5d5211     │  contract.o
 .o      decode uint32 5 (check: high bytes zero)   │  (Solang code)
 .o      inc: value += by                           │
 .o        __sys_sload(0)      ──► guest: map[0] = 5
 .o        __sys_sstore(0, 10) ──► guest: map[0] = 10
 .o      __sys_return("")      ──► guest: commit (false, "", {0:10}); halt
host   reads public values: before {0:5} → after {0:10}
```
Real output (with logging): 188,019 cycles. Without the `println!`s it was
about 46,800.

---

## 7. What a proof gives you, and what it doesn't

```
 vkey   = fingerprint of the program (guest + contract.o)
 proof  = "this program produced these public values"
 verify = right vkey? valid maths? hash(public values) == hash inside proof?
```
- Change one public byte (claim `slot0 = 99`) and the hash no longer matches,
  so it's **rejected**.
- Use a different contract or guest and it's a different vkey, so it's
  **rejected**.
- **Not proven yet:** that `{slot0 = 5}` was the real chain state. The host
  chose it; the proof only makes it public. Fixing that is the next step.

---

## 8. Why this is useful

1. **No interpreter cost.** Running EVM bytecode in a zkVM means
   interpreting every opcode; that overhead is what `evm-tax` measures. Here
   Solidity compiles straight to RISC-V, so SP1 proves the contract's own
   instructions. This setup lets you measure that difference on the same
   contract.
2. **Solidity developers don't need to learn Rust.** They keep their
   language and get zk-provable execution.
3. **One compiler, several machines.** Only the syscall layer differs between
   r55 (RISC-V `ecall`s) and SP1 (Rust functions). Codegen, ABI and storage
   layout are shared.
4. **Each piece is checked separately:** the object's shape (`tests/riscv.rs`),
   behaviour in SP1 (the host's asserts), and every storage access (the guest
   trace).

---

## 9. Honest limitations (say these before you're asked)

- **The starting storage is trusted.** There's no state root or Merkle proof
  yet.
- **Not implemented:** mappings and `keccak256`, `string`, `bytes`, dynamic
  arrays, events, `msg.sender` and `msg.value`, external calls, and contract
  creation from a contract. Using them hits `unimplemented!`.
- **No gas metering.** A transaction that would run out of gas on a chain
  succeeds here.
- **One contract per guest build**, copied in by hand as `contract.o`.
- **The Solang heap is a fixed 64 KB.**
- **Tooling:** it needs an external `llc` with RISC-V, and LLVM 16 bitcode is
  fed to a newer `llc`, which prints harmless feature warnings. `target/riscv`
  must be rebuilt after switching from the r55 branch.
- **No proof generated yet** on this machine; it needs more RAM, a GPU, or
  Succinct's prover network.

---

## 10. Next steps, in order

1. **Get a real proof:** run `--prove` on a bigger machine, or with
   `.network()`.
2. **Commit hashes instead of full storage**, e.g. `hash(storage before)` and
   `hash(storage after)`, to keep public values small.
3. **State proofs.** The host sends Merkle proofs for each slot it uses. The
   guest checks them against a known state root and commits the old and new
   roots. Then "storage before" is proven, not just stated.
4. **More of Solidity:** `keccak256` (SP1 has a precompile for it), then
   mappings, `msg.sender`, events, and dynamic types in the ABI.
5. **Measure:** cycles for the same contract compiled natively versus run by
   revm inside SP1. That's the `evm-tax` comparison.
6. **Later:** gas metering (Solang adds an instruction counter to each block)
   and cross-contract calls.

---

## 11. Likely review questions

- **Why RV64IM and not RV64IMAC?** SP1 implements RV64IM. `tests/riscv.rs`
  checks that there are no `a` or `c` instructions.
- **Why an object file, and not a full program?** The SP1 crate already
  provides `_start`, the memory layout, the allocator and the syscalls.
  Writing our own would be fragile, so the guest links us in.
- **Why 4 × u64 limbs?** A 256-bit value doesn't fit in a register, and RISC-V
  passes function arguments in 64-bit registers.
- **Why a custom ABI encoder?** Solang's shared one copies its in-memory
  layout, which is little-endian and packed. The Ethereum ABI is big-endian
  32-byte words.
- **What makes a revert correct?** `__sys_revert` commits `BEFORE`, so none of
  the writes count.
- **What stops the host lying about the result?** The proof: the public
  values are hashed into it. The host *can* still lie about the starting
  state, which is next step 3.
- **Is the r55 branch going to be merged?** No. The r55 work stays a separate
  branch; code is copied from it where useful.

---

## 12. Demo commands

```bash
cd ~/void/projects/prova
./prova setup                                  # build Solang and its RV64IM stdlib
./prova compile contracts/incrementer.sol      # -> build/contract.o
./prova inspect                                # symbols, ISA, disassembly
./prova run 'deploy 5' 'inc(uint32) 5' 'get()' 'nope()'
./prova prove 'deploy 5' 'inc(uint32) 5'       # needs a stronger machine

cd ~/void/projects/solang && cargo test --test riscv
```
