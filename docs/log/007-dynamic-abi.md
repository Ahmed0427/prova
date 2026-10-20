# 007: `string` and `bytes` in the Ethereum ABI

## 1. What and why

An ERC-20's `name()` and `symbol()` return `string`, and real ERC-20s
revert with `require(cond, "message")`, whose revert data is
`Error(string)`. Both failed to compile (`cannot ABI encode string`). This
adds Ethereum head/tail encoding and decoding of `string` and `bytes` for
top-level arguments and return values, and makes selector-prefixed data
(calldata, errors) correct. The new decoder also behaves like solc on
malformed calldata (empty revert data, trailing bytes ignored), which fixes
the trailing-calldata deviation recorded in 002.

## 2. Decision

**(a) Where to implement the tuple encoding.** Solang's shared driver
(`abi::abi_encode/abi_decode`) encodes arguments one after another, which
fits SCALE and Borsh but not the Ethereum ABI, where a dynamic value puts an
offset in the head and its data in a tail after all heads.

| Option | For | Against |
|---|---|---|
| A. Bend the per-value `encode()` hooks (track a tail offset inside the encoder) | no shared code touched | the driver computes the size and advances offsets itself; the tail would still have to be threaded through it |
| **B. A riscv branch at the top of the driver's `abi_encode`/`abi_decode`, into riscv functions that do the whole tuple** | all the logic in `riscv/encoding.rs`; static values still go through the existing per-value encoder | two small `if ns.target == Riscv` branches in shared code |

Chosen: **B**.

**(b) Selectors.** Codegen passes a selector as the *first argument* of an
ordinary encode (`[bytes4 selector, args...]`) in five places: external
calls (two), `abi.encodeWith*`/`encodeCall`, `Error(string)` and custom
errors. SCALE writes `bytes4` as 4 raw bytes, so that works there; the
Ethereum ABI writes a `bytes4` as a 32-byte word, so the selector would be
wrong, and a genuine `abi.encode(bytes4(x), …)` looks the same.

| Option | For | Against |
|---|---|---|
| A. Treat a leading `bytes4` literal as a selector on riscv | no shared change | wrong for `abi.encode(bytes4(0x…), …)` |
| **B. Say it: `abi_encode_with_selector(selector, args)`, used at those five sites** | no guessing; also needed by phase 2 calls | touches shared codegen: a new `TargetCodegen` method whose default is exactly the old behaviour (insert, then `self.abi_encode`), and a module function for `revert.rs` (no target object there) |

Chosen: **B**. Other targets produce the same CFG as before (checked
below).

**(c) Calldata checks.** The shared `BufferValidator` reverts with
`Panic(0)` on short calldata and on trailing bytes; solc reverts with empty
data on malformed calldata and ignores trailing bytes. The riscv decoder
checks the head size once, then each dynamic value's offset and length, all
reverting with empty data, and never checks for trailing bytes. A new
`BufferValidator::assume_verified()` stops the shared per-value reader from
re-checking what the head check covered.

**(d) What is in scope.** `string` and `bytes` as top-level values.
Dynamic arrays (`uint256[]`) and structs containing dynamic values would
need recursive head/tail encoding; an ERC-20 needs neither. They still stop
compilation with `unimplemented`. `abi.encodePacked` now stops compilation
instead of silently producing the non-packed encoding (it ignored `packed`).

**(e) Storage strings** (`string public name;` set in the constructor) are
not implemented: they need solc's short/long string slot layout. The ERC-20
uses `string public constant`, which is enough for `name()`/`symbol()`.

**(f) Host syntax.** Values on the command line and in tests can now be
`"text"` (a `string`) and `hex"00ff"` (a `bytes`), Solidity literal syntax;
the host tuple-encodes the argument list, so no function signature parsing
is needed. `returns` and `log … data` use the same encoder.

## 3. How it works

Compiler (`src/codegen/targets/riscv/encoding.rs`):

```
abi_encode(selector?, args):
    head = Σ (dynamic(a) ? 32 : static_size(a))
    len_i = length(a_i) for each dynamic a_i
    size = prefix(4 if selector) + head + Σ (32 + pad32(len_i))
    buf = new bytes(size)                  # zero-filled: padding is free
    if selector: buf[0..4] = selector
    tail = head
    for a in args (offsets relative to after the selector):
        if static:  encode(a) at head position; pos += static_size
        else:       word(tail) at pos; word(len) at tail; memcpy(a) at tail+32
                    tail += 32 + pad32(len); pos += 32

abi_decode(buf, size, types):
    require(head <= size)                               # else revert ""
    for t in types:
        if static: v = read(t) at pos                   # canonical-value check as before
        else:
            off = read u256 at pos;     require(off <= size - 32)
            len = read u256 at off;     require(len <= size - 32 - off)
            v = new t(len); memcpy(v, buf + off + 32, len)
    # no trailing-bytes check
```

Shared codegen:

```
abi::abi_encode(args, packed):   if riscv: packed ? unimplemented : riscv::abi_encode(None, args)
abi::abi_decode(...):            if riscv: riscv::abi_decode(...)
abi_encode_with_selector(sel, args):   riscv ? riscv::abi_encode(Some(sel), args)
                                              : abi_encode([sel] ++ args)        # old behaviour
TargetCodegen::abi_encode_with_selector  default: self.abi_encode([sel] ++ args); riscv overrides
call sites: encode_many_with_selector, two external-call payloads, Error(string), custom errors
```

Runtime: unchanged.

## 4. Real code

Solang, `src/codegen/targets/abi.rs`: the riscv branches.

```diff
 ) -> (Expression, Expression) {
+    if ns.target == Target::Riscv {
+        if packed {
+            unimplemented!("the riscv target cannot abi.encodePacked yet");
+        }
+        return crate::codegen::targets::riscv::encoding::abi_encode(loc, None, args, ns, vartab, cfg);
+    }
     let mut encoder = create_encoder(ns, packed);
```

Solang, `src/codegen/targets/abi.rs`: the selector entry point (other targets unchanged).

```diff
+pub(crate) fn abi_encode_with_selector(loc, selector, mut args, ns, vartab, cfg) -> (Expression, Expression) {
+    if ns.target == Target::Riscv {
+        return crate::codegen::targets::riscv::encoding::abi_encode(loc, Some(selector), args, ns, vartab, cfg);
+    }
+    args.insert(0, selector);
+    abi_encode(loc, args, ns, vartab, cfg, false)
+}
```

Solang, `src/codegen/expression.rs`: a call site.

```diff
-                args.insert(0, selector);
-
-                let (payload, _) = target.abi_encode(loc, args, ns, vartab, cfg, false);
+                let (payload, _) =
+                    target.abi_encode_with_selector(loc, selector, args, ns, vartab, cfg);
```

Solang, `src/codegen/revert.rs`: `Error(string)` when not constant-foldable.

```diff
-                    .or_else(|| abi_encode(loc, args, ns, vartab, cfg, false).0.into())
+                    .or_else(|| {
+                        let args = args[1..].to_vec();
+                        let selector = self.selector_expression(ns);
+                        abi_encode_with_selector(loc, selector, args, ns, vartab, cfg).0.into()
+                    })
```

Solang, `src/codegen/targets/riscv/encoding.rs`: the dynamic case of the encoder.

```rust
        write(offset.clone(), tail.clone(), vartab, cfg);          // offset in the head
        write(tail.clone(), len.clone(), vartab, cfg);             // length at the tail
        cfg.add(vartab, Instr::MemCopy {
            source: arg.clone(),
            destination: Expression::AdvancePointer {
                pointer: buffer.clone().into(),
                bytes_offset: at(tail.clone().add_u32(number(WORD.into()))).into(),
            },
            bytes: len.clone(),
        });
        set_u32(tail_var, tail.clone().add_u32(number(WORD.into())).add_u32(padded(len)), vartab, cfg);
```

Solang, `src/codegen/targets/riscv/encoding.rs`: the decoder's bounds checks.

```rust
        // value is the offset of the length word: offset + 32 <= size.
        EthAbiEncoding::require(LessEqual(value, size256 - 32), ns, vartab, cfg);
        let (len, _) = encoder.read_from_buffer(buffer, &start, &Uint(256), &mut validator, ...);
        // len <= size - 32 - offset
        EthAbiEncoding::require(LessEqual(len, (size256 - 32) - value), ns, vartab, cfg);
```

prova, `host/src/abi.rs` (new): the host's tuple encoder.

```rust
pub fn encode(values: &[Value]) -> Vec<u8> {
    let (mut head, mut tail) = (Vec::new(), Vec::new());
    for value in values {
        match value {
            Value::Word(word) => head.extend_from_slice(word),
            Value::Dynamic(bytes) => {
                head.extend_from_slice(&number(32 * values.len() + tail.len()));
                tail.extend_from_slice(&number(bytes.len()));
                tail.extend_from_slice(bytes);
                tail.resize(tail.len().next_multiple_of(32), 0);
            }
        }
    }
    head.extend_from_slice(&tail);
    head
}
```

## 5. Verification

```
== mixed(uint8,string,uint256,bytes) 7 "hi" 9 hex"abcd"
  return 0x…09 | …80 | …07 | …c0 | …02 6869… | …02 abcd…
         head: 9, offset 0x80, 7, offset 0xc0; tail: len 2 "hi", len 2 abcd
== check(uint256) 30
  revert 0x08c379a0 …20 …07 746f6f20626967…          Error("too big")
== echo(string) 0x20 5            revert 0x          length past the end
== echo(string) 0x1000 / -1       revert 0x          offset past the end / huge
== echo(string) 0x20 -1           revert 0x          huge length
== echo(string)                   revert 0x          no head
== echo(string) 0x40 0 3 0x686921…   return "hi!"    non-standard offset accepted
== check(uint256) 3 4             return 3           trailing bytes ignored
```

```
$ ./prova test           all 7 test files passed (strings.test 17/17; incrementer.test
                         now also checks short calldata -> empty revert, trailing bytes ok;
                         mappings.test now calls the string-key getter byName(string))
$ cargo test --test riscv      ok. 4 passed
```

Other targets, because shared codegen changed:

```
$ cargo test --no-fail-fast --test codegen --test evm --test polkadot --test contract
codegen   ok. 2 passed      evm  ok. 6 passed
polkadot  301 passed; 1 failed: polkadot_tests::strings::bytes_storage
contract  2 passed; 1 failed: solana_contracts (invalid_space.sol diagnostics)
$ cargo test --lib       ok. 144 passed
```

Both failures are pre-existing: with this change stashed (`git stash`) they
fail the same way (`bytes_storage` panics at `src/emit/instructions.rs:774`;
`invalid_space.sol` gets an extra "does not fit into type uint64"
diagnostic).

## 6. Commits

- Solang: `b9856c6b encode string and bytes in the ethereum abi with selectors kept raw`
- prova: `c107412 test string and bytes abi, Error(string) and malformed calldata`

## 7. Open issues / follow-ups

- Dynamic arrays and nested dynamic types in the ABI.
- Storage strings/bytes (solc's short/long layout) and storage arrays.
- `abi.encodePacked`.
- `require` messages are encoded at run time on the failure path (a
  constant-foldable `Error("literal")` could be folded like `Panic` is).
