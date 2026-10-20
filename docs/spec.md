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

Interface version: **4** (weak shift helpers; several contracts per program, log 009).

## 1. Object file

| Property | Value | Status |
|---|---|---|
| Format | ELF64, little-endian, `ET_REL`, `EM_RISCV` (243) | implemented |
| ISA | RV64IM only: `e_flags == 0` (no `C`, soft-float ABI), `.riscv.attributes` arch `rv64i2p1_m2p0` without `_a`/`_c` | implemented, checked by `tests/riscv.rs` |
| Calling convention | RISC-V LP64 psABI (integer args in `a0`–`a7`, return in `a0`; 32-bit ints sign-extended to 64 bits) | implemented |
| Code model | medium (`-code-model=medium`, stdlib built with `-mcmodel=medany`), matching SP1's `riscv64im-succinct-zkvm-elf` | implemented |
| Defined global symbols | `solang_deploy`, `solang_call` | implemented |
| Defined weak symbols | `__ashlti3`, `__ashrti3`, `__lshrti3` (128-bit shift helpers LLVM calls by name; weak so that several objects link together) | implemented, checked by `tests/riscv.rs` |
| Undefined symbols | only `__sys_*` (§3) | implemented, checked by `tests/riscv.rs` |
| Everything else | internal linkage (heap, memcpy, bigint helpers, the contract's own functions) | implemented |

The three shift helpers are an implementation detail, not part of the
interface; a runtime must not call them. Every object has the same two
entry point names; a runtime that links several contracts into one program
renames them (§7).

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
| `string`, `bytes` (offset in the head; length + zero-padded data in the tail) | implemented, as top-level arguments and return values (log 007) |
| dynamic arrays `T[]`, structs or fixed arrays containing `string`/`bytes` | not implemented |
| `abi.encodePacked` | not implemented (stops compilation) |
| external function types | not planned |

Decoding is checked like solc, and every failure reverts with **empty** data:
calldata shorter than the head; a dynamic value's offset or length pointing
past the end; a non-canonical value word (`widen(decoded) != word`, enum
`>= variants`). Trailing bytes after the arguments are ignored, and any
offset inside the buffer is accepted, as in solc.

A selector is written as 4 raw bytes before the tuple (calldata, `Error`,
`Panic`, custom errors, `abi.encodeWithSelector/Signature/Call`).

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
`bool` 0/1, `address` as a uint160.

Mappings use solc's slots exactly: `m[k]` for a mapping at slot `p` is at
`keccak256(abi_word(k) ++ p)` for a value type key (`abi_word` = the 32-byte
ABI encoding: integers sign/zero-extended, `bytesN` left-aligned, `address`
right-aligned) and `keccak256(k ++ p)` for a `string`/`bytes` key, `p` as
32 big-endian bytes. Nested mappings apply this repeatedly; a struct value
takes consecutive slots from the hashed one. Dynamic array slots: not
implemented (`.push`, `.length` on storage arrays don't compile).

Since every value has its own slot, a contract made only of full-word values
and mappings (an ERC-20) has the same storage as solc gives it; one with
small values next to each other does not (solc packs them). See log 004.

### 3.3 Hashing — implemented

```c
void __sys_keccak256(const uint8_t *data, uint64_t len, uint8_t out[32]);
```

Writes Keccak-256 (Ethereum's, padding `0x01`) of `data[0..len]` to `out`,
the digest in its natural byte order. `out` has no alignment requirement.
The compiler turns the digest into the `bytes32`/`uint256` whose big-endian
bytes it is. Used for `keccak256(...)` and for mapping slots (§3.2). The SP1
guest uses SP1's patched `tiny-keccak`, so each 136-byte block is one
`KECCAK_PERMUTE` precompile call. Calls whose input is constant at compile
time (e.g. `keccak256("abc")`) are folded and never reach the runtime.

### 3.4 Transaction context — implemented

```c
void     __sys_caller      (uint8_t  out[20]);   // msg.sender
void     __sys_callvalue   (uint64_t out[4]);    // msg.value, as limbs
void     __sys_address     (uint8_t  out[20]);   // address(this)
uint64_t __sys_block_number(void);               // block.number
uint64_t __sys_timestamp   (void);               // block.timestamp, seconds
```

Addresses are 20 bytes, big-endian; `out` has no alignment requirement
for the addresses and 8-byte alignment for the limbs. Every entry point of
a function that is not `payable` (and the constructor unless it is
`payable`, including the implicit one) calls `__sys_callvalue` first and
reverts with empty data if the value is not 0, as solc does. No value is
moved: there are no balances yet, `msg.value` is only a number.

In the SP1 guest the context comes from the host (§4) and is
**unauthenticated**: no signature or chain proves the sender, block or
timestamp. The proof only makes them public (§5), so a verifier knows which
sender the result is for, but must trust whoever chose it.

### 3.5 Events — implemented

```c
void __sys_log(const uint8_t *data, uint64_t len,
               const uint8_t *topics, uint64_t n_topics);
```

One event. `topics` points to `n_topics` 32-byte words next to each other
(`n_topics ≤ 4`, unaligned); `data` is its data. The runtime records it with
the emitting contract's address. Events emitted by an execution that
reverts are discarded.

What the compiler passes, as on Ethereum: topic 0 is `keccak256` of the
event signature unless the event is `anonymous`; each `indexed` field is
one topic holding its 32-byte ABI word, in order; `data` is the ABI encoding
of the non-indexed fields, in order (empty if none). Indexed fields of
reference types (`string`, `bytes`, arrays, structs; Ethereum puts their
hash in the topic) don't compile yet.

### 3.6 End of execution — implemented

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
| unknown selector, malformed calldata (§2), value sent to a non-payable function | empty |
| `require(cond)`, `revert()` | empty |
| `assert(false)` | `Panic(1)`: `0x4e487b71` + word `0x01` |
| arithmetic overflow | `Panic(0x11)` |
| array index out of bounds | `Panic(0x32)` |
| division or modulo by zero | `Panic(0x12)` |
| `type(intN).min / -1` | `Panic(0x11)`, also inside `unchecked { }` (solc wraps there; see log 003) |
| out of heap | empty |
| `require(cond, "msg")`, `revert("msg")` | `Error(string)`: `0x08c379a0` + the ABI encoding of the message |
| custom errors `revert E(...)` | `selector(E)` + the ABI encoding of the fields |

Neither function returns. The compiler emits `unreachable` after the call.

**Revert scope.** Today there is one execution per transaction, so a revert
ends the transaction. With nested calls (planned, phase 2) `__sys_revert`
ends only the current call frame: it discards that frame's writes and
returns failure to the caller; only a top-level revert ends the transaction.

### 3.7 Planned imports

Proposed signatures. Each will be settled, with the options considered, in
its log entry.

| Import | Proposed signature | Solidity | Phase |
|---|---|---|---|
| `__sys_call` | `uint64_t (const uint8_t addr[20], const uint64_t value[4], const uint8_t *input, uint64_t len)` → 0 success, 1 revert | external calls | 2 |
| `__sys_returndata_size` / `__sys_returndata_copy` | `uint64_t (void)` / `void (uint8_t *dest, uint64_t offset, uint64_t len)` | return data of the last call | 2 |

## 4. SP1 guest input — implemented

The host writes one `prova_common::Input` with `SP1Stdin::write`; the guest
reads it with `sp1_zkvm::io::read`. `common/src/lib.rs` is the definition
for both sides. Encoding: bincode v1, default options (integers
little-endian, `Vec`/`String` as a `u64` length then the items, `bool` one
byte, `Option` one byte tag then the value, `[u8; N]` without a length,
struct fields in order).

```rust
struct Input {
    context: Context,
    deploy: Option<String>,    // Some(name): deploy that contract at context.address
                               // None: call the contract at context.address
    calldata: Vec<u8>,         // constructor args (deploy) or selector + args (call)
    state: Vec<Account>,       // the state the transaction starts from, sorted by address
}
struct Account {
    address: [u8; 20],
    code: String,              // the name of a contract linked into the guest
    storage: Vec<(Word, Word)>,// (slot, value) big-endian, sorted by slot
}
struct Context {
    caller: [u8; 20],          // msg.sender
    value: [u8; 32],           // msg.value, big-endian
    address: [u8; 20],         // address(this): the contract called or deployed
    block_number: u64,
    timestamp: u64,
}
```

Deploying where an account already has code reverts (as `CREATE` fails on
Ethereum). Calling an address without code succeeds with empty output (as a
call to an account without code does on Ethereum). A `code` name that isn't
linked into the guest is a host error: the guest panics.

Host defaults (`./prova run`, tests): caller `0x1`, value 0, address
`0xc0de`, block 1, timestamp 1; `--from`, `--value`, `--to`, `--block`,
`--timestamp` change them.

## 5. SP1 public values — implemented

The guest commits exactly one `prova_common::Public`, bincode as above, and
then calls `syscall_halt(0)`.

```rust
struct Public {
    context: Context,              // as given
    deploy: Option<String>,        // as given
    calldata: Vec<u8>,             // as given
    state_before: Vec<Account>,    // as given
    reverted: bool,
    output: Vec<u8>,               // return or revert data
    logs: Vec<Log>,                // events in emission order; empty on revert
    state_after: Vec<Account>,     // sorted by address, storage sorted by slot; = state_before on revert
}
struct Log {
    address: [u8; 20],             // the contract that emitted it
    topics: Vec<[u8; 32]>,         // ≤ 4
    data: Vec<u8>,
}
```

A proof attests: this program (guest + the linked contracts, fixed by the
verifying key) run on input `(context, deploy, calldata, state_before)`
produced `(reverted, output, logs, state_after)`. It does **not** attest
that `state_before` is real chain state or that `context` is a real
transaction. The code name in an account is meaningful only together with
the verifying key: the key fixes which object each name runs.

Planned later: hashes or state roots in place of full state and logs
(log 006).

## 6. Planned compiler-side semantics

| Item | Plan | Phase |
|---|---|---|
| mapping slot | implemented (§3.2) | 1 |
| dynamic array / string storage | as solc: length at `p`, data from `keccak256(p)` | 1 (as needed by ERC-20) |
| storage packing | decided: one slot per value, no packing (log 004) | 1 |

## 7. Multiple contracts — implemented (log 009)

Several contract objects can be linked into one program. In this repo,
`./prova compile A.sol B.sol` compiles every contract of each file and
renames each object's entry points with `llvm-objcopy`:
`solang_deploy` → `prova_deploy_NAME`, `solang_call` → `prova_call_NAME`.
Every other symbol is local or weak, so nothing else collides. The guest's
build script links all of them and generates a table `name → (deploy,
call)`; an account's `code` names its row.

Calls between contracts and the per-frame state they need are planned
(phase 2, log 010). The `__init_heap` reset at every entry point means a
re-entrant call into a contract still running would clobber its heap; this
has to be solved with them.
