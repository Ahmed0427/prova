# 004: `__sys_keccak256`, solc mapping slots, no storage packing

## 1. What and why

An ERC-20 is two mappings, and mappings need keccak. This adds one import,
`__sys_keccak256`, implemented in the guest with SP1's patched `tiny-keccak`
so the permutation runs as SP1's `KECCAK_PERMUTE` precompile instead of
~thousands of RISC-V instructions. The compiler uses it for
`keccak256(bytes)` and for mapping slots, which now follow solc exactly:
`keccak256(abi_word(key) ++ slot)`, or `keccak256(key ++ slot)` for
string/bytes keys.

## 2. Decision

**(a) `__sys_keccak256` signature.**

| Option | For | Against |
|---|---|---|
| **A. `(data, len, out: u8[32])`, digest bytes** | a hash returns bytes; the same shape fits sha256/ripemd later; nothing about Solang's integer layout leaks into the runtime | the contract byte-swaps the result (one `bswap.i256`, a few instructions) |
| B. `(data, len, out: u64[4])`, the digest as LE limbs of a uint256 | no swap in the contract | mixes "hash" with "how Solang holds a `bytes32`"; the runtime must know to interpret the digest as big-endian |
| C. A mapping-specific `__sys_mapping_slot(key, slot) -> slot` | one call per lookup | a mapping layout baked into the runtime interface; string keys and `keccak256()` still need A |

Chosen: **A**.

**(b) Where to build the mapping preimage.** Solang's generic
`Expression::Keccak256` (what Polkadot uses for mappings) hashes values in
LLVM's native, little-endian layout, which is not solc's. Options: emit
big-endian `abi.encode`-style codegen in `lower_mapping_subscript`, or keep
the default lowering (`Expression::Subscript` on a mapping) and implement the
riscv emitter's `storage_subscript`. Chosen: **the emitter**. It is all in
`src/emit/riscv/target.rs`, it has the key's type and value, and for value
keys it can hash a 64-byte stack buffer with no heap traffic.
`keccak256_hash` (the generic path, used by dynamic storage arrays) stays
unimplemented on purpose: implementing it with the native layout would give
non-solc slots silently.

**(c) Storage packing.** solc packs adjacent small values into one slot;
this target gives every value its own slot.

| Option | For | Against |
|---|---|---|
| **A. One slot per value (keep)** | simple codegen; every `sstore` is a plain write; mappings are already solc-identical, so an ERC-20 (all `uint256` + mappings) has exactly solc's layout | layouts differ from solc for contracts with small adjacent values; more slots per contract |
| B. solc packing | storage-compatible with any solc contract, so a native contract could take over an existing contract's state, and a revm comparison could use identical state | every write to a packed slot is a read-modify-write (an extra `__sys_sload`); masks and shifts in every access; substantial work in `load_slots`/`store_slots` |

Chosen: **A** for now. Nothing in phases 1-3 needs B (the benchmark's
ERC-20 is word-sized), and in a zkVM a slot costs a map lookup, not 20k gas.
B becomes necessary if native contracts must read storage written by legacy
solc contracts (phase 4 touches this). The layout is a compiler decision, not
part of the runtime interface, so it can change without touching the guest.

## 3. How it works

Compiler (`src/emit/riscv/target.rs`):

```
storage_subscript(mapping_ty, slot, key):
    if key type is string/bytes:
        buf = malloc(len(key) + 32); buf = key_bytes ++ be32(slot)
        h = keccak256(buf); free(buf)
    else:
        buf = stack [2 x i256]
        buf[0] = bswap(abi_word(key))   # int: sext, bytesN: << (256-8N), address: uint160
        buf[1] = bswap(slot)
        h = keccak256(buf, 64)
    return h                           # nested mappings and struct fields add on to it

keccak256(data, len):
    out = stack i256
    __sys_keccak256(data, len, out)
    return bswap(load out)             # digest bytes, read as a big-endian number

hash(Keccak256, data, len) = keccak256(data, len)      # keccak256(bytes) builtin
```

Guest (`guest/src/main.rs`):

```
__sys_keccak256(data, len, out):
    out = tiny_keccak::Keccak::v256(data[0..len])   # SP1 fork: KECCAK_PERMUTE per block
```

Host: `run()` now returns the execution report's syscall counts; `./prova
run` prints the precompiles used, and tests can assert them with `syscall
NAME COUNT`.

## 4. Real code

Solang, `src/emit/riscv/target.rs`: the import.

```diff
+    let keccak_ty = void_ty.fn_type(&[i8_ptr_ty.into(), i64_ty.into(), i8_ptr_ty.into()], false);
+    let keccak = bin.module.add_function("__sys_keccak256", keccak_ty, None);
+    keccak.set_linkage(Linkage::External);
```

Solang, `src/emit/riscv/target.rs`: the digest is read as a big-endian number.

```diff
+        let out = bin.build_alloca(function, i256_ty, "digest");
+        bin.builder.build_call(bin.module.get_function("__sys_keccak256").unwrap(),
+                &[data.into(), len.into(), out.into()], "").unwrap();
+        let digest = bin.builder.build_load(i256_ty, out, "digest").unwrap().into_int_value();
+        Self::bswap256(bin, digest)
```

Solang, `src/emit/riscv/target.rs`: a mapping key as its ABI word.

```diff
+        match ty {
+            Type::Int(_) => bin.builder.build_int_s_extend_or_bit_cast(int(), i256_ty, "key").unwrap(),
+            // bytesN is left-aligned.
+            Type::Bytes(n) => { ... build_left_shift(word, 256 - 8 * n) }
+            _ => {
+                Self::check_single_slot(bin, ty);
+                Self::to_slot_word(bin, ty, value)
+            }
+        }
```

Solang, `src/emit/riscv/target.rs`: value-type keys hash a stack buffer.

```diff
+        if !matches!(key_ty, Type::String | Type::DynamicBytes) {
+            let preimage = bin.build_alloca(function, i256_ty.array_type(2), "preimage");
+            let words = [Self::abi_word(bin, &key_ty, index), slot];
+            for (i, word) in words.into_iter().enumerate() {
+                let dest = ... build_gep(i256_ty.array_type(2), preimage, [0, i]) ...;
+                bin.builder.build_store(dest, Self::bswap256(bin, word)).unwrap();
+            }
+            return Self::keccak256(bin, function, preimage, i32_ty.const_int(64, false));
+        }
```

Solang, `tests/riscv.rs`: a mapping contract is RV64IM and imports only `__sys_*`.

```diff
+    let (_, undefined) = global_symbols(&elf);
+    assert!(undefined.contains("__sys_keccak256"), "{undefined:?}");
+    assert!(undefined.iter().all(|name| name.starts_with("__sys_")), "{undefined:?}");
```

prova, `guest/Cargo.toml`: SP1's tiny-keccak.

```diff
+tiny-keccak = { version = "2.0.2", features = ["keccak"] }
+# SP1's tiny-keccak runs the permutation as the keccak precompile.
+[patch.crates-io]
+tiny-keccak = { git = "https://github.com/sp1-patches/tiny-keccak", tag = "patch-2.0.2-sp1-6.2.0" }
```

prova, `guest/src/main.rs`: the import.

```diff
+#[no_mangle]
+extern "C" fn __sys_keccak256(data: *const u8, len: u64, out: *mut [u8; 32]) {
+    let data = bytes(data, len);
+    let mut keccak = Keccak::v256();
+    keccak.update(data);
+    unsafe { keccak.finalize(&mut *out) };
```

prova, `host/src/main.rs`: precompile counts from the execution report.

```diff
+    let syscalls = report.syscall_counts.iter()
+        .map(|(code, count)| (format!("{code:?}"), *count))
+        .filter(|(name, count)| *count > 0 && !IO_SYSCALLS.contains(&name.as_str()))
+        .collect();
```

## 5. Verification

The expected slots were computed with an independent pure-Python
Keccak-256 (scratch; checked against `keccak("") = c5d24601…a470` and the
`transfer(address,uint256)` selector `a9059cbb`):

```
byNumber[7]      dc686ec4a0ff239c70e7c7c36e8f853eced3bc8618f48d2b816da2a74311237e
byAddress[abcd]  01faefabee0c1b2040e07e2151ec1b32c8f39cc788b6c6991fb3408cb4910eac
nested[a][b]     21c324ff47714b60a3b024b87c239034ab80bd75118640c4845df95dc9fd79b3
flags[cafebabe]  9a56c5317ed65e3c0ed48e542a46afb2f4709763bb4c7238a6bd3a0788a984a7
bySigned[-1]     2e8de2577e7c560a9913fd732cd5ba1f61f809b10c283800da9499091ac562a5
points[1]        3e5fec24aa4dc4e5aee2e025e51e1392c72a2500577559fae9665c6d52bd6a31
byName[alice]    a478e43b88f0bf0999792486cdb85c44a79ae39db2de1d4da061e8d73f2e6ba9
```

The guest's trace shows the solc preimages, e.g.:

```
== setNested(address,address,uint256) 0xa 0xb 9
  keccak 0x00…0a00…03 -> 0x9dcb9783…8d53
  keccak 0x00…0b9dcb9783…8d53 -> 0x21c324ff…79b3
  sstore 0x21c324ff47714b60a3b024b87c239034ab80bd75118640c4845df95dc9fd79b3 <- 0x9
== setAlice(uint256) 42
  keccak 0x616c696365 00…07 -> 0xa478e43b…6ba9
== setNumber(uint256,uint256) 7 100
  cycles          369702
  precompiles     KECCAK_PERMUTE x1
```

```
$ ./prova test
tests/arith.test: 16/16 steps passed
tests/incrementer.test: 6/6 steps passed
tests/mappings.test: 20/20 steps passed
tests/storage_types.test: 17/17 steps passed
all 4 test files passed

$ cargo test --test riscv
test result: ok. 3 passed
```

## 6. Commits

- Solang: `9fa1a001 add keccak256 syscall and solc mapping slots to riscv target`
- prova: `3efc545 add keccak256 syscall on the SP1 precompile and mapping tests`

## 7. Open issues / follow-ups

- The `string` key's mapping getter (`mapping(string => …) public`) needs
  dynamic ABI decoding; the test uses a constant key until that lands.
- Dynamic storage arrays and storage strings (`.push`, `.length`, `string
  public name` in storage) are unimplemented; with (b) they will hash
  `be32(slot)` the same way.
- `abi.encodePacked` uses the non-packed encoder on this target (the
  encoder ignores `packed`); `keccak256(abi.encodePacked(...))` would be
  wrong. To check and fix with the ABI work.
- Every guest trace line formats 32-byte words; benchmarks must disable it.
