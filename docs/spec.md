# Contract runtime interface

A contract compiled with `solang compile --target riscv` is an RV64IM ELF
relocatable object (`contract.o`). It talks to its environment **only**
through the functions listed here: it exports two entry points and imports a
set of `__sys_*` functions. Anything that runs the object (the SP1 guest in
this repo today, an r55-style VM later) is a *runtime*, and must provide the
imports with exactly these signatures and semantics.

Status tags: **implemented** (the compiler emits it and the SP1 guest
provides it), **planned** (proposed here; the signature is final only once
its log entry in `docs/log/` lands and the tag changes).

Interface version: **0** (phase 0 baseline).

## 1. Object file

| Property | Value | Status |
|---|---|---|
| Format | ELF64, little-endian, `ET_REL`, `EM_RISCV` (243) | implemented |
| ISA | RV64IM only: `e_flags == 0` (no `C`, soft-float ABI), `.riscv.attributes` arch `rv64i2p1_m2p0` without `_a`/`_c` | implemented, checked by `tests/riscv.rs` |
| Calling convention | RISC-V LP64 psABI (integer args in `a0`–`a7`, return in `a0`; 32-bit ints sign-extended to 64 bits) | implemented |
| Code model | medium (`-code-model=medium`, stdlib built with `-mcmodel=medany`), matching SP1's `riscv64im-succinct-zkvm-elf` | implemented |
| Defined global symbols | `solang_deploy`, `solang_call`, plus `__ashlti3`, `__ashrti3`, `__lshrti3` (128-bit shift helpers LLVM calls by name) | implemented |
| Undefined symbols | only `__sys_*` (§3) | implemented, checked by `tests/riscv.rs` |
| Everything else | internal linkage (heap, memcpy, bigint helpers, the contract's own functions) | implemented |

The three shift helpers are an implementation detail, not part of the
interface; a runtime must not call them. Linking two contracts into one
program makes them, and the entry points, collide: see §7.

### Memory

- The contract's heap is a 64 KiB static array inside the object
  (`stdlib/heap.c`). Every entry point re-initialises it (`__init_heap`).
  Running out of heap calls `__sys_revert(NULL, 0)`.
- The contract uses the caller's stack. Its depth is not bounded by the
  contract.
- SP1 traps on misaligned loads and stores; the compiler keeps its own
  accesses aligned, and pointers it passes to imports are aligned for their
  type (8 bytes for `u64` limbs).

## 2. Entry points (exported)

Both take the input buffer and its length, and **never return**: they always
end in `__sys_return` or `__sys_revert`.

```c
void solang_deploy(const uint8_t *input, uint32_t len);   // implemented
void solang_call  (const uint8_t *input, uint32_t len);   // implemented
```

| Entry point | Input | What it does |
|---|---|---|
| `solang_deploy` | the ABI-encoded constructor arguments, **no** selector | `__init_heap`; run the state variable initializers; decode the arguments and run the constructor (if any); `__sys_return(NULL, 0)` |
| `solang_call` | 4-byte selector, then the ABI-encoded arguments | `__init_heap`; `len < 4` or an unknown selector reverts with empty data; decode the arguments, run the function, ABI-encode its return values; `__sys_return(out, out_len)` |

There is no fallback or receive function dispatch yet (planned with
phase 2 calls).

### ABI

Calldata and return data are the Ethereum ABI.

| Type | Status |
|---|---|
| value types (`uintN`, `intN`, `bool`, `address`, `bytesN`, enums, user value types) | implemented |
| fixed-size arrays and structs of the above (inline) | implemented |
| `string`, `bytes`, dynamic arrays (offset + length encoding) | planned (phase 1) |
| external function types | not planned |

Decoding is strict, like solc: each argument word must be canonical
(`widen(decoded) == word`, enum `< variants`), and calldata shorter than the
static size of the arguments reverts with empty data.

**Deviation from solc:** calldata *longer* than the arguments reverts with
`Panic(0)`; solc ignores trailing bytes. (Known, not yet fixed.)

## 3. Imports (`__sys_*`)

### 3.1 Conventions

- **256-bit words by value** are passed as four `uint64_t` limbs, least
  significant first: `w = l0 + l1·2^64 + l2·2^128 + l3·2^192`. In the SP1
  guest, `to_word`/`to_limbs` convert them to 32 big-endian bytes.
- **256-bit words by pointer** (outputs) point to `uint64_t[4]` in the same
  limb order, 8-byte aligned. In memory this is the little-endian `i256`
  Solang uses.
- **Byte strings** are `(const uint8_t *ptr, uint64_t len)`. `ptr` may be
  `NULL` when `len == 0`. The memory belongs to the contract and is valid
  only during the call; a runtime that keeps the bytes must copy them.
- **Addresses** (planned imports) are 20 bytes, big-endian, as Solang keeps
  `address` in memory.
- Imports are plain function calls in the SP1 guest (Rust `extern "C"`), not
  SP1 syscalls. Another runtime may implement them as `ecall` stubs.
- An import other than `__sys_return`/`__sys_revert` returns normally.

### 3.2 Storage — implemented

```c
void __sys_sload (uint64_t k0, uint64_t k1, uint64_t k2, uint64_t k3,
                  uint64_t out[4]);
void __sys_sstore(uint64_t k0, uint64_t k1, uint64_t k2, uint64_t k3,
                  uint64_t v0, uint64_t v1, uint64_t v2, uint64_t v3);
```

Storage is the contract's own map from 256-bit slot to 256-bit word. A slot
never written reads as 0. `sstore` of 0 is a write of 0 (the guest keeps the
slot in the map with value 0; a runtime may delete it, and the two are
equivalent).

Layout chosen by the compiler (not the runtime's concern, documented for
tests): one slot per value, no packing; state variables in declaration
order from slot 0; structs and fixed arrays take consecutive slots, one per
value inside. Words hold the value as solc holds an unpacked value: integers,
enums and `bytesN` zero-extended from their own width (so `int64 -42` is
`0xffffffffffffffd6`, and `bytes4 0xcafebabe` is `0xcafebabe`, right-aligned),
`bool` 0/1, `address` as a uint160. Mapping and dynamic array slots: planned
(phase 1, §6).

### 3.3 End of execution — implemented

```c
_Noreturn void __sys_return(const uint8_t *data, uint64_t len);
_Noreturn void __sys_revert(const uint8_t *data, uint64_t len);
```

- `__sys_return`: the execution succeeded; `data` is the ABI-encoded return
  value (empty for `deploy` and for functions with no returns).
- `__sys_revert`: the execution failed; **all** storage writes of this
  execution are discarded. `data` is the revert data:

| Cause | Revert data |
|---|---|
| unknown selector, short calldata, non-canonical argument | empty |
| `require(cond)`, `revert()` | empty |
| `assert(false)` | `Panic(1)`: `0x4e487b71` + word `0x01` |
| arithmetic overflow | `Panic(0x11)` |
| array index out of bounds | `Panic(0x32)` |
| division or modulo by zero | **does not revert** (known bug): RISC-V `div`/`rem` by zero give all-ones/the dividend instead of trapping, and the compiler emits no check, so `6 / 0` on `uint8` returns 255. solc reverts with `Panic(0x12)`. To fix in phase 1. |
| out of heap | empty |
| `require(cond, "msg")`, `revert("msg")`, custom errors with dynamic fields | not compiled yet: needs `string` encoding (planned, phase 1) |

Neither function returns. The compiler emits `unreachable` after the call.

**Revert scope.** Today there is one execution per transaction, so a revert
ends the transaction. With nested calls (planned, phase 2) `__sys_revert`
ends only the current call frame: it discards that frame's writes and
returns failure to the caller; only a top-level revert ends the transaction.

### 3.4 Planned imports

Proposed signatures. Each will be settled, with the options considered, in
its log entry.

| Import | Proposed signature | Solidity | Phase |
|---|---|---|---|
| `__sys_keccak256` | `void (const uint8_t *data, uint64_t len, uint8_t out[32])` — digest in its natural byte order | `keccak256(...)`, mapping slots | 1 |
| `__sys_log` | `void (const uint8_t *data, uint64_t len, const uint8_t *topics, uint64_t n_topics)` — `topics` is `n_topics × 32` big-endian bytes, `n_topics ≤ 4` | `emit E(...)` | 1 |
| `__sys_caller` | `void (uint8_t out[20])` | `msg.sender` | 1 |
| `__sys_callvalue` | `void (uint64_t out[4])` | `msg.value` | 1 |
| `__sys_address` | `void (uint8_t out[20])` | `address(this)` | 1 |
| `__sys_block_number` | `uint64_t (void)` | `block.number` | 1 |
| `__sys_timestamp` | `uint64_t (void)` | `block.timestamp` | 1 |
| `__sys_call` | `uint64_t (const uint8_t addr[20], const uint64_t value[4], const uint8_t *input, uint64_t len)` → 0 success, 1 revert | external calls | 2 |
| `__sys_returndata_size` / `__sys_returndata_copy` | `uint64_t (void)` / `void (uint8_t *dest, uint64_t offset, uint64_t len)` | return data of the last call | 2 |

`__sys_caller` and `__sys_callvalue` are already *declared* in the compiler
with these signatures, but nothing calls them, so they do not appear in the
object. `msg.value` is currently compiled as the constant 0 (used for the
non-payable check), and `msg.sender` does not compile.

## 4. SP1 guest input — implemented

The host writes three values with `SP1Stdin::write`; the guest reads them
with `sp1_zkvm::io::read`, in this order. Each is bincode (v1, default
options: lengths as `u64` LE, `bool` as one byte, `[u8; N]` without a
length).

| # | Field | Rust type | Meaning |
|---|---|---|---|
| 1 | `deploy` | `bool` | call `solang_deploy` (true) or `solang_call` |
| 2 | `calldata` | `Vec<u8>` | the entry point's input |
| 3 | `storage` | `Vec<([u8; 32], [u8; 32])>` | starting storage, `(slot, value)` big-endian; any order, a repeated slot keeps its last value |

Planned (phase 1): a transaction context (sender, value, contract address,
block number, timestamp) supplied by the host. It will be **unauthenticated**:
the proof will make it public but not prove it.

## 5. SP1 public values — implemented

The guest `commit`s six values, bincode as above, in this order. The host's
`Public::read`/`Public::write` must match it exactly.

| # | Field | Rust type | Committed |
|---|---|---|---|
| 1 | `deploy` | `bool` | before execution |
| 2 | `calldata` | `Vec<u8>` | before execution |
| 3 | `storage_before` | `Vec<(Word, Word)>` | before execution |
| 4 | `reverted` | `bool` | at `__sys_return`/`__sys_revert` |
| 5 | `output` | `Vec<u8>` | return or revert data |
| 6 | `storage_after` | `Vec<(Word, Word)>` | the final map, sorted by slot; equal to `storage_before` on revert |

Then the guest calls `syscall_halt(0)`. A proof attests: this program (guest
+ linked contract, fixed by the verifying key) run on input
`(deploy, calldata, storage_before)` produced `(reverted, output,
storage_after)`. It does **not** attest that `storage_before` is real chain
state.

Planned (phase 1): the transaction context and the logs (or their hash).
Planned later: hashes or state roots in place of full storage.

## 6. Planned compiler-side semantics

| Item | Plan | Phase |
|---|---|---|
| mapping slot | `keccak256(pad32(key) ++ pad32(slot))`, as solc | 1 |
| dynamic array / string storage | as solc: length at `p`, data from `keccak256(p)` | 1 (as needed by ERC-20) |
| storage packing | open decision (solc packing vs one slot per value); see the phase 1 log | 1 |

## 7. Multiple contracts — planned (phase 2)

Today the object exports fixed names, so one program can link only one
contract. Phase 2 gives each contract unique symbol names, adds an address →
contract registry in the guest, `__sys_call`, and a storage journal per call
frame. The `__init_heap` reset at every entry point means a re-entrant call
into a contract still running would clobber its heap; this has to be solved
in phase 2 too.
