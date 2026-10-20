# 006: events through `__sys_log`

## 1. What and why

Until now the riscv target dropped events silently (`RiscvEventEmitter::emit`
was an empty TODO). An ERC-20 must emit `Transfer` and `Approval`. This
emits Ethereum-style events: topic 0 = keccak256 of the signature (unless
`anonymous`), indexed fields as topics, the other fields ABI-encoded as data.
They reach the guest through one import, `__sys_log`; the guest records them
with the contract address, drops them on revert, and commits them in the
public values.

## 2. Decision

**(a) The import.**

| Option | For | Against |
|---|---|---|
| **A. `__sys_log(data, len, topics, n_topics)`, topics as `n × 32` bytes** | one import for 0-4 topics; topics stay bytes (they are hashes or ABI words) | the contract copies the topics next to each other first |
| B. `__sys_log0..4`, like EVM `LOG0..LOG4`, topics as limb arguments | no copy | five imports; LOG4 = 2 + 16 register arguments, beyond the 8 argument registers, so spills to the stack anyway |

Chosen: **A** (as proposed in the spec).

**(b) Logs in the public values: in full or a hash?**

| Option | For | Against |
|---|---|---|
| **A. Commit the logs in full** (`Vec<Log>` in `Public`) | a verifier reads the events straight from the public values; consistent with storage, which is also committed in full today; tests check them directly | public values grow with the events (SP1 hashes them; ~a few cycles per byte) |
| B. Commit `keccak256(encoding of the logs)` | constant size | the consumer needs the logs from somewhere else and must recompute the exact encoding; nothing gains from it until storage is also hashed |
| C. Both | | the size of A plus the work of B |

Chosen: **A** now. Replacing full storage and full logs with commitments
(state roots, a receipts-style hash) is one later change for both, listed in
the spec. An ERC-20 transfer's log is 3 topics + 32 bytes of data, ~150
bytes.

**(c) What a log's address is.** `context.address` (the running contract).
In phase 2 this becomes the current call frame's address.

**(d) Indexed reference types** (`string indexed name`): Ethereum hashes
them into the topic. Not needed for an ERC-20; they stop compilation with an
`unimplemented` message rather than emitting something wrong.

## 3. How it works

Compiler, codegen (`src/codegen/targets/riscv/events.rs`, new):

```
emit E(args):
    topics = []
    if !E.anonymous: topics += [const keccak256("E(type,...)")]   # computed by the compiler
    data_fields = []
    for (arg, field) in zip(args, E.fields):       # evaluated in source order
        v = eval(arg)
        if field.indexed: topics += [abi_encode(v)]   # one 32-byte word; value types only
        else:             data_fields += [v]
    data = abi_encode(data_fields)                 # empty bytes if none
    Instr::EmitEvent { data, topics }
```

Compiler, emit (`src/emit/riscv/target.rs`):

```
emit_event(data, topics):
    buf = stack [32 * max(n,1) x i8]
    for i, t in topics: memcpy(buf + 32 i, bytes(t), 32)
    __sys_log(bytes(data), len(data), buf, n)
```

Guest:

```
__sys_log(data, len, topics, n):
    assert n <= 4
    LOGS.push(Log { address: context.address, topics: split(topics, 32), data })
finish(reverted, ...):
    logs = reverted ? [] : LOGS
    commit(Public { ..., output, logs, storage_after })
```

Host: prints each log; test scripts get `log [SIG] TOPIC... [data WORD...]`
(a token with `(` is hashed into a topic) and `logs N`.

## 4. Real code

Solang, `src/codegen/targets/riscv/events.rs`: topics and data.

```rust
        if !event.anonymous {
            topics.push(Expression::AllocDynamicBytes {
                ..., initializer: Some(self.selector(contract_no)),   // keccak256(signature)
            });
        }
        // Arguments are evaluated in order, whether they become topics or data.
        for (arg, field) in self.args.iter().zip(event.fields.iter()) {
            let value = expression(arg, cfg, contract_no, Some(func), self.ns, vartab, opt, target);
            if !field.indexed {
                data.push(value);
                continue;
            }
            if value.ty().is_reference_type(self.ns) {
                unimplemented!("the riscv target cannot index an event field of type `{}` yet", ...);
            }
            topics.push(abi_encode(&loc, vec![value], self.ns, vartab, cfg, false).0);
        }
```

Solang, `src/codegen/targets/riscv/mod.rs`: the emitter replaces the no-op.

```diff
-pub(crate) struct RiscvEventEmitter;
-impl EventEmitter for RiscvEventEmitter {
-    fn emit(...) {
-        // TODO: events are dropped until the Log syscall is wired up
-    }
...
-        Box::new(RiscvEventEmitter)
+        Box::new(RiscvEventEmitter { args, ns, event_no })
```

Solang, `src/emit/riscv/target.rs`: the call.

```diff
+        for (i, topic) in topics.iter().enumerate() {
+            let dest = ... build_gep(i8, buf, [32 * i]) ...;
+            ... __memcpy(dest, bin.vector_bytes(*topic), 32) ...
+        }
+        bin.builder.build_call(bin.module.get_function("__sys_log").unwrap(),
+            &[bin.vector_bytes(data).into(), data_len.into(), buf.into(),
+              i64_ty.const_int(topics.len() as u64, false).into()], "")
```

prova, `common/src/lib.rs`: the log type and the new public field.

```diff
+pub struct Log {
+    pub address: Address,
+    pub topics: Vec<Word>,
+    pub data: Vec<u8>,
+}
 ...
     pub output: Vec<u8>,
+    /// The events emitted, in order; none after a revert.
+    pub logs: Vec<Log>,
```

prova, `guest/src/main.rs`: the import and the revert rule.

```diff
+extern "C" fn __sys_log(data: *const u8, len: u64, topics: *const u8, n_topics: u64) {
+    assert!(n_topics <= 4, "an event has at most 4 topics");
+    let topics: Vec<Word> = bytes(topics, 32 * n_topics).chunks(32)
+        .map(|topic| topic.try_into().unwrap()).collect();
+    ...
+    unsafe { LOGS.push(Log { address: input().context.address, topics, data }) };
+}
 ...
+    let logs = if reverted { Vec::new() } else { unsafe { core::mem::take(&mut LOGS) } };
```

## 5. Verification

```
== --from 0xa11ce fire()
  log    [0xd89287…9590] 0x…01 …01                                  Plain(1, true)
  log    [0x8a13fbc5…4dab, 0xa11ce, 0xb0b] 0x…05                    Indexed(sender, 0xb0b, 5)
  log    [0xc7386e6c…088d, 0xff…fe, 0xcafebabe00…] 0x…07 …c0de      Mixed(-2, 0xcafebabe, 7, this)
  log    [0x9] 0x…0a                                                Anon (anonymous: no topic 0)
  log    [0x3db2a12a…cf37] 0x                                       Empty()
== fireThenRevert()
  log    [0xd89287…9590] …            (recorded, then dropped)
  revert 0x
  reverted        true          (no `log` lines in the public values)
```

Topic 0 values agree with solc and with the independent keccak:

```
$ solc --hashes contracts/events.sol
3db2a12a…cf37: Empty()
8a13fbc5…4dab: Indexed(address,address,uint256)
c7386e6c…088d: Mixed(int8,bytes4,uint64,address)
d8928737…9590: Plain(uint256,bool)
```

A deliberately wrong script fails as it should:

```
  FAIL  fire() (badlog.test:2)
        5 events, expected 1
        event 0 is [0xd892…] 0x…01…01, expected [0xd892…] 0x…02…01
  FAIL  fire() (badlog.test:4)
        5 events, expected 4
```

```
$ ./prova test        all 6 test files passed (events.test: 3/3)
$ cargo test --test riscv      test result: ok. 4 passed (new: events_import_log)
```

## 6. Commits

- Solang: `6adc7552 emit ethereum-style events through a log syscall on riscv`
- prova: `809fc8c add log syscall and commit events in the public values`

## 7. Open issues / follow-ups

- Indexed `string`/`bytes`/struct/array fields (hash as topic).
- Logs and storage are committed in full; replace both with commitments
  later.
- No bloom filter or receipt; nothing here needs one.
