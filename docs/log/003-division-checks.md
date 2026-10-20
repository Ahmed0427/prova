# 003: division by zero and `min / -1` revert like solc

## 1. What and why

Found while writing the spec (002): on the riscv target `6 / 0` returned 255
instead of reverting. Solang emits a bare LLVM `udiv`/`sdiv`/`urem`/`srem`
for integers up to 64 bits, and relies on the target trapping (wasm does).
RISC-V never traps: `div` by zero gives all-ones and `rem` gives the
dividend. The same applies to `type(intN).min / -1`, which solc reports as
`Panic(0x11)` but RISC-V (and Solang's wide `sdivmod` helpers, on every
target) return `min`. A contract computing e.g. a share `amount / total`
with `total == 0` would have silently succeeded.

## 2. Decision

**Zero divisor:** guard only the ≤64-bit path, only on riscv. The >64-bit
paths call `udivmod128/256`, `sdivmod128/256`, which already return an error
code that the emitter turns into `Panic(0x12)`.

**`min / -1`:** solc checks it in checked code and wraps in `unchecked { }`.
Codegen's `SignedDivide` (and the AST's `Divide`) have no `unchecked` flag;
only add/sub/mul carry `overflowing`.

| Option | For | Against |
|---|---|---|
| A. Add `overflowing` to `Divide`/`SignedDivide` through sema and codegen | exact solc semantics | ~24 match sites across all targets, the optimizer passes and the LIR converter; a cross-target change on a target branch |
| **B. Always check on riscv** | small, riscv-only; checked code (the default, and ERC-20s) is exactly solc | inside `unchecked { }` we revert where solc returns `min` |
| C. Leave it, document | nothing to do | silent wrong result in checked code |

Chosen: **B**. Reverting where solc wraps is the safe direction, and
`unchecked` division by -1 of `min` is vanishingly rare. Recorded as a
deviation in the spec. Modulo needs nothing: `min % -1` is 0 on RISC-V,
as in solc.

## 3. How it works

Compiler (`src/emit/expression.rs`), in each of the four division arms:

```
bits = width(left)
if bits <= 64: riscv_divisor_check(right)          # riscv only
if SignedDivide: riscv_signed_division_check(left, right)   # riscv only, every width
... existing code (helper call for > 64 bits, plain div otherwise)

riscv_divisor_check(d):
    if d == 0: __sys_revert(Panic(0x12))
riscv_signed_division_check(a, d):
    if a == 1 << (bits-1) and d == -1: __sys_revert(Panic(0x11))
```

Runtime: unchanged (it only sees `__sys_revert`).

## 4. Real code

Solang, `src/emit/expression.rs`: the call sites (same in all four arms).

```diff
             let bits = left.get_type().get_bit_width();
 
+            if bits <= 64 {
+                riscv_divisor_check(target, bin, right, function, loc);
+            }
+            riscv_signed_division_check(target, bin, left, right, function, loc);   // SignedDivide only
+
             if bits > 64 {
```

Solang, `src/emit/expression.rs`: the zero check.

```diff
+/// RISC-V `div` and `rem` don't trap on a zero divisor, so the narrow
+/// divisions that the helpers above 64 bits don't cover check it here.
+fn riscv_divisor_check<'a, T: TargetRuntime<'a> + ?Sized>(...) {
+    if bin.ns.target != Target::Riscv {
+        return;
+    }
+    let is_zero = ... build_int_compare(IntPredicate::EQ, divisor, divisor.get_type().const_zero(), ...);
+    ... build_conditional_branch(is_zero, bail_block, ok_block);
+    bin.builder.position_at_end(bail_block);
+    let (revert_out, revert_out_len) = bin.panic_data_const(PanicCode::DivisionByZero);
+    target.assert_failure(bin, revert_out, revert_out_len);
+    bin.builder.position_at_end(ok_block);
+}
```

Solang, `src/emit/expression.rs`: the overflow check.

```diff
+    let min = ... build_left_shift(ty.const_int(1, false), ty.const_int(bits - 1, false), "min");
+    let is_min = ... build_int_compare(IntPredicate::EQ, dividend, min, "dividend_is_min");
+    let is_minus_one = ... build_int_compare(IntPredicate::EQ, divisor, ty.const_all_ones(), ...);
+    let overflows = ... build_and(is_min, is_minus_one, "overflows");
+    ...
+    let (revert_out, revert_out_len) = bin.panic_data_const(PanicCode::MathOverflow);
+    target.assert_failure(bin, revert_out, revert_out_len);
```

prova: `contracts/arith.sol` and `tests/arith.test` (new).

## 5. Verification

Before the fix (probe, log 002): `div(uint8,uint8) 6 0` → `return 0x…ff`.
Wide signed overflow before the fix:

```
== sdiv128(int128,int128) -170141183460469231731687303715884105728 -1
  return 0xffffffffffffffffffffffffffffffff80000000000000000000000000000000
```

After:

```
$ ./prova test tests/arith.test
  ok    udiv8(uint8,uint8) 6 3
  ok    udiv8(uint8,uint8) 6 0            (reverts, Panic(0x12))
  ok    umod64 / sdiv32 / smod16 / udiv256 / smod128: value, and by zero -> Panic(0x12)
  ok    sdiv32(int32,int32) -2147483648 -1          (reverts, Panic(0x11))
  ok    sdiv128(int128,int128) -2^127 -1            (reverts, Panic(0x11))
  ok    sdiv128(int128,int128) -2^127 1             returns -2^127
  ok    smod16(int16,int16) -32768 -1               returns 0
tests/arith.test: 16/16 steps passed

$ ./prova test        all 3 test files passed
$ cargo test --test riscv     test result: ok. 2 passed
$ cargo clippy        no warnings in files this branch touches
```

## 6. Commits

- Solang: `17bdfed7 check riscv division by zero and signed division overflow`
- prova: filled in by the next commit.

## 7. Open issues / follow-ups

- `min / -1` inside `unchecked { }` reverts (solc: `min`). Fixing it means
  option A.
- Other places that may rely on a wasm trap: none found for arithmetic
  (add/sub/mul overflow checks are explicit in codegen; shifts don't trap in
  either). Worth a sweep for `unreachable`-as-trap in the emitter.
