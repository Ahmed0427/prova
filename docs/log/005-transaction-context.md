# 005: transaction context (`msg.sender`, `msg.value`, `address(this)`, block info)

## 1. What and why

An ERC-20 needs `msg.sender`. This adds five imports for the transaction
context, a `Context` the host passes to the guest (with `--from`, `--value`,
`--to`, `--block`, `--timestamp`), and commits it in the public values. The
context is unauthenticated: the host chooses it and the proof only makes it
public. On the way, the riscv dispatcher gets solc's non-payable check (it
had none, and `msg.value` was hard-wired to 0), and the input/public-values
types move into one crate shared by guest and host.

## 2. Decision

**(a) One import per field, or one context struct?**

| Option | For | Against |
|---|---|---|
| **A. One import per field** (`__sys_caller`, `__sys_callvalue`, `__sys_address`, `__sys_block_number`, `__sys_timestamp`) | mirrors EVM opcodes (CALLER, CALLVALUE, …), so an r55-style runtime maps them 1:1; adding `tx.origin`, `chainid`, … later adds imports without breaking any; per call frame (phase 2) the runtime just answers from the current frame | a function call per use (a few cycles each) |
| B. `__sys_context(out: *Context)` | one call | a struct layout in the interface: every new field is a breaking change, and the contract and runtime must agree on padding |

Chosen: **A**. It was also what Solang had already declared
(`__sys_caller(ptr)`, `__sys_callvalue(ptr)`) and what the spec proposed.
Addresses go out as 20 big-endian bytes (Solang's in-memory address, no
conversion); the value as `u64[4]` limbs like every other word output;
block number and timestamp as a returned `u64` (Solang types them `uint64`).

**(b) Non-payable check.** solc reverts when a non-payable function (or
constructor, including the implicit one) receives value. Once `msg.value`
is real, leaving this out would let value into every function. Added to the
riscv dispatcher, as Polkadot's dispatcher does. Cost: one
`__sys_callvalue` per call.

**(c) The guest/host format.** The guest committed six values and the host
re-read them field by field (`Public::read/write`), a match kept by hand.
Options: keep it and add fields to both; or a `common` crate with `Input`,
`Context`, `Public` used by both, written and read as one value each.
Chosen: **the crate** (`prova-common`, `no_std` + serde derive). bincode of a
struct is the concatenation of its fields, so the bytes are the same kind of
format, but there is one definition. The commit moves from "inputs first,
outputs at the end" to one commit at the end, which is equivalent: a run
that never reaches `__sys_return`/`__sys_revert` has no public values at all.

**(d) CLI syntax for the context.** `--from X` before the steps sets the
default for all of them; at the start of a step string it applies to that
step. One rule serves the command line and test files (where every step is
a line).

## 3. How it works

Compiler:

```
builtin(Sender)      = { buf: [20 x i8]; __sys_caller(&buf); load buf }
builtin(GetAddress)  = { buf; __sys_address(&buf); &buf }        # codegen loads it
builtin(Value)       = value_transferred = { w: i256; __sys_callvalue(&w); w }
builtin(BlockNumber) = __sys_block_number()
builtin(Timestamp)   = __sys_timestamp()

dispatch, each public function case (and deploy):
    if function not payable (no constructor counts as not payable):
        if msg.value != 0: revert ""
    decode args; call; encode
```

Guest:

```
main: INPUT = read::<Input>(); STORAGE = INPUT.storage; entry(calldata)
__sys_caller(out)       : *out = INPUT.context.caller
__sys_callvalue(out)    : *out = to_limbs(INPUT.context.value)
__sys_address(out)      : *out = INPUT.context.address
__sys_block_number()    : INPUT.context.block_number
__sys_timestamp()       : INPUT.context.timestamp
finish(reverted, output, storage_after):
    commit(Public { INPUT's fields..., reverted, output, storage_after }); halt
```

Host: `parse_step(text, defaults)` takes leading `--opt value` pairs into the
step's `Context`; `main` takes single-token `--opt value` pairs before the
steps as the defaults.

## 4. Real code

Solang, `src/emit/riscv/target.rs`: the imports.

```diff
-    let caller_ty = void_ty.fn_type(&[i8_ptr_ty.into()], false);
-    let caller = bin.module.add_function("__sys_caller", caller_ty, None);
-    ...
+    // The transaction context: each writes to its out pointer, or returns.
+    for name in ["__sys_caller", "__sys_callvalue", "__sys_address"] {
+        let ty = void_ty.fn_type(&[i8_ptr_ty.into()], false);
+        bin.module.add_function(name, ty, None).set_linkage(Linkage::External);
+    }
+    for name in ["__sys_block_number", "__sys_timestamp"] {
+        let ty = i64_ty.fn_type(&[], false);
+        bin.module.add_function(name, ty, None).set_linkage(Linkage::External);
+    }
```

Solang, `src/emit/riscv/target.rs`: the builtins.

```diff
+        match kind {
+            Builtin::Sender => bin.builder
+                .build_load(bin.address_type(), address("__sys_caller"), "caller").unwrap(),
+            // Codegen loads it: GetAddress is a pointer to the address.
+            Builtin::GetAddress => address("__sys_address").into(),
+            Builtin::Value => self.value_transferred(bin).into(),
+            Builtin::BlockNumber => number("__sys_block_number"),
+            Builtin::Timestamp => number("__sys_timestamp"),
+            _ => unimplemented!("the riscv target has no syscall for {kind:?} yet"),
+        }
```

Solang, `src/emit/riscv/target.rs`: `msg.value` is no longer a constant.

```diff
-    fn value_transferred<'b>(&self, contract: &Binary<'b>) -> IntValue<'b> {
-        let i256_ty = contract.context.custom_width_int_type(256);
-        i256_ty.const_zero()
+    fn value_transferred<'b>(&self, bin: &Binary<'b>) -> IntValue<'b> {
+        ...
+        let out = bin.build_alloca(function, i256_ty, "value");
+        bin.builder.build_call(bin.module.get_function("__sys_callvalue").unwrap(), &[out.into()], "")
```

Solang, `src/codegen/targets/riscv/dispatch.rs`: the non-payable check.

```diff
+        // solc's implicit constructor is not payable either.
+        self.abort_if_value_transfer(constructor.is_none_or(|(_, cfg)| cfg.nonpayable));
 ...
         self.cfg.set_basic_block(case_bb);
+        self.abort_if_value_transfer(self.all_cfg[func_no].nonpayable);
 ...
+    fn abort_if_value_transfer(&mut self, nonpayable: bool) {
+        if !nonpayable { return; }
+        self.add(Instr::BranchCond {
+            cond: Expression::NotEqual { left: Builtin::Value, right: 0 },
+            true_block: got_value, false_block: no_value });
+        self.cfg.set_basic_block(got_value);
+        self.add(Instr::AssertFailure { encoded_args: None });
+        self.cfg.set_basic_block(no_value);
+    }
```

Solang, `tests/riscv.rs`: the incrementer now imports `__sys_callvalue` too.

```diff
         set(&[
+            "__sys_callvalue",
             "__sys_return",
```

prova, `common/src/lib.rs` (new): one definition of the format.

```rust
pub struct Context { pub caller: Address, pub value: Word, pub address: Address,
                     pub block_number: u64, pub timestamp: u64 }
pub struct Input  { pub context: Context, pub deploy: bool, pub calldata: Vec<u8>, pub storage: Storage }
pub struct Public { pub context: Context, pub deploy: bool, pub calldata: Vec<u8>,
                    pub storage_before: Storage, pub reverted: bool, pub output: Vec<u8>,
                    pub storage_after: Storage }
```

prova, `guest/src/main.rs`: one input, one commit, the context imports.

```diff
-    let deploy: bool = read();
-    let calldata: Vec<u8> = read();
-    let storage: Vec<(Word, Word)> = read();
-    commit(&deploy); commit(&calldata); commit(&storage);
+    let input: Input = read();
 ...
+#[no_mangle]
+extern "C" fn __sys_caller(out: *mut Address) {
+    unsafe { *out = input().context.caller };
+}
 ...
-    commit(&reverted); commit(&data); commit(&storage);
+    commit(&Public { context: input.context, deploy: input.deploy, calldata: input.calldata,
+                     storage_before: input.storage, reverted, output, storage_after });
```

prova, `host/src/main.rs`: context options.

```diff
+fn set_option(context: &mut Context, name: &str, value: &str) -> Result<bool, String> {
+    match name {
+        "--from" => context.caller = parse_address(value)?,
+        "--value" => context.value = parse_word(value)?,
+        "--to" => context.address = parse_address(value)?,
+        "--block" => context.block_number = parse_u64(value)?,
+        "--timestamp" => context.timestamp = parse_u64(value)?,
+        _ => return Ok(false),
+    }
```

## 5. Verification

```
$ ./prova run --from 0xa11ce deploy 'whoami()' '--from 0xb0b whoami()' 'me()' '--to 0x1234 me()' \
    '--block 77 --timestamp 1700000000 when()' '--value 5 deposit()' '--value 7 deposit()' \
    'deposits()' '--value 1 whoami()' 'onlyOwner()' '--from 0xb0b onlyOwner()'
== deploy                     return 0x            (owner = 0xa11ce)
== whoami()                   return 0x…0a11ce
== --from 0xb0b whoami()      return 0x…000b0b
== me()                       return 0x…00c0de
== --to 0x1234 me()           return 0x…001234
== --block 77 --timestamp 1700000000 when()
                              return 0x…4d 0x…6553f100     (77, 1700000000)
== --value 5 deposit(), --value 7 deposit(), deposits()   return 0x…0c
== --value 1 whoami()         revert 0x            (not payable)
== onlyOwner()                return 0x…01
== --from 0xb0b onlyOwner()   revert 0x
```

```
$ ./prova test
tests/arith.test: 16/16, tests/context.test: 16/16, tests/incrementer.test: 6/6,
tests/mappings.test: 20/20, tests/storage_types.test: 17/17
all 5 test files passed
$ cargo test --test riscv      test result: ok. 3 passed
$ cargo clippy (both repos)    no warnings in touched files
```

## 6. Commits

- Solang: `877b7537 read msg.sender, msg.value, address(this) and block info from syscalls`
- prova: `4e5dbe6 add transaction context syscalls, host options and shared io types`

## 7. Open issues / follow-ups

- The context is unauthenticated. Authenticating it means verifying a
  signed transaction (ecrecover over the tx hash) inside the guest, and
  proving the block header; out of scope.
- No balances: `msg.value` is a number only; `address.balance`,
  `transfer`, `send` don't compile.
- `tx.origin`, `block.chainid`, `block.coinbase`, `gasleft()` hit
  `unimplemented`; each would be one more import under (a).
- In phase 2 the context becomes per call frame (`msg.sender` of a nested
  call is the calling contract).
