# 002: contract runtime interface spec (`docs/spec.md`)

## 1. What and why

The goal is that a compiled contract talks to its environment only through
`__sys_*` imports, so another runtime (r55-style) can run the same object.
That needs a written interface, not just "whatever `guest/src/main.rs`
does". `docs/spec.md` states the object format, the two entry points, every
`__sys_*` import with its exact C signature, how 256-bit words are passed,
revert semantics and revert data, and the SP1 guest's input and public
values byte layout. Each item is tagged implemented or planned.

## 2. Decision

**Planned imports: fix signatures now, or only when implemented?**

| Option | For | Against |
|---|---|---|
| A. Leave planned imports unspecified | nothing to revise | no shared target for phase 1/2 |
| **B. Propose signatures now, tagged "planned", final when their log entry lands** | the reviewer sees the whole surface early; each phase-1 entry argues against a concrete baseline | some may change |
| C. Freeze all signatures now | stable | would freeze choices (context as one struct or several calls, how `__sys_call` returns data) before the code shows the trade-offs |

Chosen: **B**. The proposals follow the conventions already in use: words
by value as 4 LE `u64` limbs, word outputs through a `u64[4]` pointer, bytes
as `(ptr, len)`, plus one new one, addresses as 20 big-endian bytes (Solang's
in-memory layout, so no conversion in the contract).

The spec records current behaviour faithfully, including the deviations from
solc found while writing it (below), rather than describing the intended
behaviour.

Also stated as normative: imports are plain C-ABI calls, not SP1 syscalls;
memory passed to an import belongs to the contract and must be copied if
kept; a never-written slot reads 0; `__sys_return`/`__sys_revert` never
return. Public values: the six-field bincode layout, and what a proof does
and does not attest.

## 3. How it works

Documentation only; no code changed. The facts came from:

```
entry points, imports   <- src/emit/riscv/mod.rs, target.rs (declare_syscalls, sload/sstore, split_256)
ABI support/strictness  <- src/codegen/targets/riscv/encoding.rs + runs below
revert data             <- runs of a probe contract (below)
bincode layout          <- sp1-lib io.rs: commit = bincode::serialize_into, read = bincode::deserialize
input/public values     <- guest/src/main.rs, host/src/main.rs (Public::read/write)
```

## 4. Real code

None (docs only). The key new text, `docs/spec.md` §3.1:

```
- 256-bit words by value are passed as four uint64_t limbs, least
  significant first: w = l0 + l1·2^64 + l2·2^128 + l3·2^192.
- 256-bit words by pointer (outputs) point to uint64_t[4] in the same
  limb order, 8-byte aligned.
- Byte strings are (const uint8_t *ptr, uint64_t len). ptr may be NULL
  when len == 0. ... a runtime that keeps the bytes must copy them.
```

## 5. Verification

Revert data, observed with a probe contract (scratch, not committed) under
`./prova run`:

```
== a()      require(false)                  revert 0x
== b(uint8) 10    x = v + 250 (overflow)    revert 0x4e487b71…11      Panic(0x11)
== c()      revert()                        revert 0x
== d()      assert(false)                   revert 0x4e487b71…01      Panic(1)
== idx(uint256) 3   uint8[3] out of bounds  revert 0x4e487b71…32      Panic(0x32)
```

`require(false, "nope")` does not compile:

```
not implemented: the RISC-V target cannot ABI encode `string` yet: only static types are supported
```

Two deviations from solc found:

```
== div(uint8,uint8) 6 0          return 0x…ff        solc: revert Panic(0x12)
== div(uint8,uint8) 6 3 7        revert 0x4e487b71…00  solc: returns 2 (ignores trailing bytes)
```

`./prova test`: 2 files, 23/23 steps passed. Solang unchanged.

## 6. Commits

- prova: filled in by the next commit.
- Solang: none.

## 7. Open issues / follow-ups

- **Division/modulo by zero doesn't revert.** Proposed as the first fix in
  phase 1: emit a zero check before `div`/`rem` on the riscv target and
  revert with `Panic(0x12)`, with a test.
- Trailing calldata reverts with `Panic(0)`. Lower priority; solc accepts it.
- `__ashlti3`/`__ashrti3`/`__lshrti3` are exported; with several contracts
  in one guest they collide (phase 2).
- `__init_heap` at every entry point breaks re-entrancy into a running
  contract (phase 2).
