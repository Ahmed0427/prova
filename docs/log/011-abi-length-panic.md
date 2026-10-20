# 011: oversized ABI lengths revert with `Panic(0x41)`

## 1. What and why

Found by the phase 3 benchmark harness (012), which runs every step
natively and as solc bytecode in revm and compares the public values:
`echo(string)` with calldata claiming a string of length `2^256 − 1`
reverted with empty data natively, but with `Panic(0x41)` (out of memory)
under solc. solc's decoder checks that a length can be allocated before it
checks the bounds. The riscv decoder now does the same.

## 2. Decision

| Option | For | Against |
|---|---|---|
| A. Keep reverting with empty data | simplest; malformed calldata is rejected either way | revert data differs from solc for this input |
| **B. `len > 2^64 − 1` → `Panic(0x41)`, before the bounds check** | the rule solc applies first (`array_allocation_size`); a clean, runtime-independent threshold | solc *also* panics for lengths just below 2^64, where `free memory pointer + length` overflows; that band depends on solc's memory layout and isn't mirrored |
| C. Use our own allocation limit (`len ≥ 2^32`, Solang vectors are 32-bit) | "can't allocate" by our own rule | disagrees with solc for every length in `[2^32, 2^64)`, which revert with empty data there |

Chosen: **B**; the remaining band is in the spec and in a test comment.

## 3. How it works

```
abi_decode, for each string/bytes argument:
    off = head word;  require(off <= size - 32)            else revert ""
    len = word at off
    if len > 2^64 - 1: revert Panic(0x41)                  # new, as solc
    require(len <= size - 32 - off)                        else revert ""
```

## 4. Real code

Solang, `src/codegen/targets/riscv/encoding.rs`:

```diff
+        // As solc: a length that could never be allocated is Panic(0x41),
+        // checked before the bounds.
+        let allocatable = cfg.new_basic_block("abi_length_allocatable".into());
+        let too_long = cfg.new_basic_block("abi_length_too_long".into());
+        cfg.add(vartab, Instr::BranchCond {
+            cond: Expression::LessEqual { signed: false, left: len, right: u64::MAX (as uint256) },
+            true_block: allocatable,
+            false_block: too_long,
+        });
+        cfg.set_basic_block(too_long);
+        assert_failure(&Codegen, SolidityError::Panic(PanicCode::OutOfMemory), ns, cfg, vartab);
+        cfg.set_basic_block(allocatable);
```

prova, `tests/strings.test`: both sides of the threshold.

```diff
-# A huge length:
-echo(string) 0x20 -1
-    reverts
-    output 0x
+echo(string) 0x20 -1
+    reverts
+    output 0x4e487b71…41
+echo(string) 0x20 0x10000000000000000
+    reverts
+    output 0x4e487b71…41
+echo(string) 0x20 0xffffffffffffffff
+    reverts
+    output 0x
```

## 5. Verification

Before (from `./prova bench tests/strings.test`, 012):

```
MISMATCH echo(string) 0x20 -1
  native: reverted true output 0x
  revm:   reverted true output 0x4e487b71…41
```

After: that step agrees; the one remaining difference is the documented
band:

```
MISMATCH echo(string) 0x20 0xffffffffffffffff
  native: reverted true output 0x
  revm:   reverted true output 0x4e487b71…41
```

```
$ ./prova test             all 11 test files passed (strings.test 20/20)
$ cargo test --test riscv  ok. 6 passed
```

(The test suite ran on the working tree that also holds 012's changes,
which are committed right after this.)

## 6. Commits

- Solang: `c4fc132b revert with Panic(0x41) for abi lengths above 2^64-1 as solc does`
- prova: `162cf16 revert with Panic(0x41) on oversized abi lengths, as solc`

## 7. Open issues / follow-ups

- The band below 2^64 (see Decision).
