# 010: calls between contracts, with per-frame reverts

## 1. What and why

Contracts linked into one guest (009) can now call each other. A call runs
in its own frame; a revert in it undoes only that frame's (and its
callees') storage writes and events and returns failure to the caller;
only a revert in the outermost frame ends the transaction. This needs: call
imports with return data, a storage journal per frame, a way for
`__sys_return`/`__sys_revert` (which never return) to hand control back to
the caller, a heap that survives re-entry, and the compiler's call, `try`/
`catch` and low-level call paths for the riscv target.

## 2. Decision

**(a) How a frame ends.** The contract calls `__sys_return`/`__sys_revert`
from anywhere in its own call stack and they never return.

| Option | For | Against |
|---|---|---|
| A. Compiler propagates status: every internal function returns a code, callers check it (Solana target style) | no assembly | changes every function's signature and every call site in the compiler; slower everywhere, not only on calls |
| B. Separate stacks per frame (coroutines) | clean isolation | allocating and switching stacks in the guest is more assembly than A or C, and doesn't remove the need to abandon the callee's frames |
| **C. setjmp/longjmp in the runtime**: save ra/sp/s0-s11 before entering a frame, restore them in `__sys_return`/`__sys_revert` | no compiler change; the object's interface stays "these never return"; another runtime (a VM with one instance per frame) needs nothing like it | ~40 lines of RISC-V assembly; frames that are abandoned must not own anything needing `Drop` (true for contract frames and the guest's `__sys_*` frames, which copy what they need first) |

Chosen: **C**. From Rust's side `prova_enter` is an ordinary function that
returns once, either when the entry point "returns" (a bug, reported) or
when `prova_leave` lands in its epilogue with the status, so no
`returns_twice` function is called from Rust. The registers it restores are
exactly the callee-saved set of the LP64 ABI, so the Rust code around the
call is consistent afterwards.

**(b) Undo.** A journal of `(address, slot, old value)` per storage write,
and per frame the journal length and event count at its start. A revert
pops the journal back and truncates the events. The alternative, copying
the storage map per frame, costs O(storage) per call.

**(c) Re-entrancy and the heap.** Every entry point calls `__init_heap`,
which reset the contract's static heap: a re-entrant call (A → B → A) would
have overwritten A's live allocations. Options: reject re-entrancy in the
guest (not Ethereum's semantics); give each frame its own heap (needs
per-frame heap memory and compiler/stdlib changes); **make `__init_heap`
idempotent**: a 3-line stdlib change. Chosen: the last. A frame's
allocations are not freed when it ends (a 64 KiB heap per contract per
transaction; running out reverts).

**(d) The imports.** One per EVM call opcode (`__sys_call`,
`__sys_staticcall`, `__sys_delegatecall`), like the context imports (005),
rather than one import with a `kind` argument; status 0/1 rather than
Polkadot's codes. Return data as EVM `RETURNDATASIZE/RETURNDATACOPY`
(`__sys_returndata_size/copy`) rather than an output buffer passed to the
call, since the caller can't know the size up front.

Inside the compiler, codegen's call paths expect a status variable with
Polkadot's codes (0 success, 2 callee reverted) for its `RetCodeCheck` and
`try_catch`. The riscv emitter maps 0/1 to 0/2 so those shared paths work
on riscv unchanged (gated `is_polkadot() || Riscv`), instead of writing
riscv copies of them.

**(e) Semantics chosen to match Ethereum**: a call to an address without
code succeeds with empty return data; >1024 frames fails the call; a static
frame can't `sstore`, `log` or send value; `delegatecall` runs the callee's
code on the caller's storage, address, sender and value. Not matched: no
balances (value is only a number), and Solang calls `view` functions with
CALL where solc uses STATICCALL.

## 3. How it works

Guest:

```
run(entry, input, address, caller, value, static):          # one frame
    if depth >= 1024: return (reverted, "")
    push Frame { address, caller, value, static,
                 journal_len = |journal|, logs_len = |logs|, resume }
    status = prova_enter(entry, input, &frame.resume)        # asm: save regs, call entry
    frame = pop
    if status == reverted:
        undo journal back to frame.journal_len; logs.truncate(frame.logs_len)
    return (status, take(output))

__sys_return(data) / __sys_revert(data):
    output = copy(data)
    prova_leave(&top_frame.resume, 0 or 1)                   # asm: restore regs, return from prova_enter

__sys_sstore(slot, v):  if static: revert frame
                        journal.push((frame.address, slot, storage[(frame.address, slot)]))
                        storage[(frame.address, slot)] = v
__sys_call(addr, value, input):        call(code: addr, address: addr, caller: frame.address, value)
__sys_staticcall(addr, input):         call(addr, addr, frame.address, 0, static = true)
__sys_delegatecall(addr, input):       call(code: addr, address: frame.address,
                                            caller: frame.caller, value: frame.value)
call(...): if no code at `code`: returndata = ""; return 0
           (reverted, out) = run(CODES[code].call, copy(input), ...)
           returndata = out; return reverted

main: (reverted, output) = run(deploy or call entry, calldata, context.address, context.caller, ...)
      commit Public (state_after = state_before if reverted)
```

Compiler:

```
external_call(address, payload, value, call type, success?):
    status = __sys_call(&address, &value, payload, len)        # or staticcall / delegatecall
    if success var: success = status * 2                      # codegen's 0 / 2 convention
    else if status != 0:                                       # plain other.f(): bubble up
        data = return_data(); __sys_revert(data)
return_data():
    n = __sys_returndata_size(); v = new bytes(n); __sys_returndata_copy(v, 0, n)

codegen (shared, now also for riscv):
    other.f(x):  success var + RetCodeCheck (0 -> decode returns, 2 -> bubble, else revert "")
    a.call(d):   (success == 0, return data)
    try/catch:   catch Error(string)/Panic: decode from returndata + 4 (riscv: a tuple of its own)
heap.c:  __init_heap() { if ready: return; ready = true; ... }
sema:    `.staticcall` allowed on riscv (was EVM only)
encoding: contract-typed values cast via `address` (runtime_cast has no contract <-> uint arm)
```

## 4. Real code

prova, `guest/src/main.rs`: the trampoline.

```rust
core::arch::global_asm!(
    ".globl prova_enter",
    "prova_enter:", // a0 = entry, a1 = input, a2 = len, a3 = resume
    "sd ra, 0(a3)",
    "sd sp, 8(a3)",
    "sd s0, 16(a3)",
    ...
    "sd s11, 104(a3)",
    "mv t0, a0",
    "mv a0, a1",
    "mv a1, a2",
    "jalr t0",
    "call prova_entry_returned",
    ".globl prova_leave",
    "prova_leave:", // a0 = resume, a1 = status
    "ld ra, 0(a0)",
    "ld sp, 8(a0)",
    ...
    "mv a0, a1",
    "ret",
);
```

prova, `guest/src/main.rs`: a frame, and undoing it.

```rust
    let status = unsafe { prova_enter(entry, input.as_ptr(), input.len() as u32, resume) };

    let world = self::world();
    let frame = world.frames.pop().unwrap();
    let reverted = status != 0;
    if reverted {
        while world.journal.len() > frame.journal_len {
            let (address, slot, old) = world.journal.pop().unwrap();
            match old {
                Some(value) => world.storage.insert((address, slot), value),
                None => world.storage.remove(&(address, slot)),
            };
        }
        world.logs.truncate(frame.logs_len);
    }
    (reverted, core::mem::take(&mut world.output))
```

prova, `guest/src/main.rs`: leaving a frame.

```rust
fn leave(reverted: bool, data: *const u8, len: u64) -> ! {
    let world = world();
    world.output = bytes(data, len).to_vec();
    ...
    let resume: *const Resume = &world.frames.last().unwrap().resume;
    unsafe { prova_leave(resume, reverted as u64) }
}
```

Solang, `stdlib/heap.c`: re-entry keeps the heap.

```diff
+// Every entry point calls this, also when a contract is entered again while it
+// is still running (A calls B calls A): only the first call may set it up.
+static bool riscv_heap_ready;
+
 void __init_heap()
 {
+    if (riscv_heap_ready)
+        return;
+    riscv_heap_ready = true;
```

Solang, `src/emit/riscv/target.rs`: the call and the status.

```diff
+        let name = match ty {
+            CallTy::Regular => { ... args.push(value.into()); "__sys_call" }
+            CallTy::Static => "__sys_staticcall",
+            CallTy::Delegate => "__sys_delegatecall",
+        };
 ...
+        if let Some(success) = success {
+            ...
+            *success = bin.builder.build_int_mul(code, i32_type.const_int(2, false), "status").unwrap().into();
+            return;
+        }
+        // plain other.f(): pass the callee's revert on
+        ... build_conditional_branch(reverted, bubble, done) ...
+        let data = self.return_data(bin, function);
+        self.assert_failure(bin, bin.vector_bytes(data.into()), data_len);
```

Solang, `src/codegen/expression.rs`: riscv shares Polkadot's status handling.

```diff
-                let success = ns.target.is_polkadot().then(|| vartab.temp_name("success", &Type::Uint(32)));
+                let success = (ns.target.is_polkadot() || ns.target == Target::Riscv)
+                    .then(|| vartab.temp_name("success", &Type::Uint(32)));
 ...
-                if ns.target.is_polkadot() {
+                if ns.target.is_polkadot() || ns.target == Target::Riscv {
                     polkadot::RetCodeCheckBuilder::default()
```

Solang, `src/codegen/targets/polkadot/try_catch.rs`: catch data is a tuple after the selector.

```diff
+            // In the Ethereum ABI the data after the selector is a tuple of its own.
+            let expr = if ns.target == Target::Riscv {
+                let data = Expression::AdvancePointer { pointer: buffer.clone().into(), bytes_offset: 4 };
+                let size = ret_data_len - 4;
+                abi_decode(&Codegen, &data, std::slice::from_ref(&param_ty), ns, vartab, cfg, Some(size))[0].clone()
+            } else {
+                let types = &[Type::Bytes(4), param_ty.clone()];
+                abi_decode(&Codegen, &buffer, types, ns, vartab, cfg, None)[1].clone()
+            };
```

Solang, `src/codegen/targets/riscv/encoding.rs`: contract-typed values (a latent
bug: no earlier test passed a contract as an argument).

```diff
             Type::Address(_) | Type::Contract(_) => {
                 return Expression::Cast {
                     loc: Codegen,
                     ty: Uint(256),
-                    expr: expr.clone().into(),
+                    expr: Expression::Cast { loc: Codegen, ty: Type::Address(false), expr: expr.clone().into() }.into(),
                 }
```

## 5. Verification

The required scenario, A calls B, B reverts, A catches it and continues, B's
writes are gone (from `tests/calls.test`, guest trace):

```
== --to 0xa tryRevert(uint256) 9
  sstore 0x1 <- 0x2                                  A: steps = 2
  call 0xb 0xf016619e…09
    sstore 0x0 <- 0x9                                B: value = 9
    log    [0xdf7a95ae…425b] 0x…09                   B: Set(9)
    revert 0x08c379a0…63616c6c65652072657665727465640…   Error("callee reverted")
  log    [0x6a85348b…abb5] …"callee reverted"        A: Caught(reason)
  call 0xb 0x3fa4f245                                A reads target.value()
    sload  0x0 -> 0x7                                B's write is gone
  return (false, 7)
  log             0xa [Caught…]                      only A's event is public
  state after     0xa caller {0x0 = 0xb, 0x1 = 0x2}
                  0xb callee {0x0 = 0x7}
```

Re-entrancy (A → B → A) with A's heap and storage intact:

```
== --to 0xa reenter()
  sstore 0x1 <- 0x5
  call 0xb …
    sstore 0x0 <- 0x4
    call 0xa 0x68110b2f
      sload  0x1 -> 0x5
      sstore 0x1 <- 0x69
  sload  0x1 -> 0x69
  sstore 0x1 <- 0x73
```

```
$ ./prova test tests/calls.test
  callSet -> 14, Set(7) from 0xb; tryRevert(9) -> (false, 7), only Caught;
  tryRevert(3) -> (true, 3); rawRevert -> (false, Error bytes); uncaught -> error "callee reverted",
  whole tx undone; who() -> (0xa, 0xb); reenter; staticRead -> (true, 4);
  staticWrite -> false (sstore in a static call); delegateSet -> A's slot 0 = 0xdead
tests/calls.test: 12/12 steps passed
$ ./prova test        all 10 test files passed
$ cargo test --test riscv      ok. 6 passed (new: calls_import_call_syscalls)
$ cargo test --no-fail-fast --lib --test codegen --test evm --test polkadot --test contract
  lib 144 ok, codegen 2 ok, evm 6 ok, polkadot 301/302, contract 2/3
  (the two failures are the pre-existing ones from 007)
```

The setter in the test callee only reverts for `v >= 5`: with an
unconditional `revert`, Solang's optimizer removed the write before it, and
the test would not have exercised the journal.

## 6. Commits

- Solang: `00911b36 add calls between riscv contracts with return data and try/catch`
- prova: `2e30b75 add calls between contracts with per-frame journals in the guest`

## 7. Open issues / follow-ups

- `new C()` (contract creation from a contract): needs a `__sys_create`
  and the guest to pick an address (CREATE/CREATE2 rules).
- Balances and value transfer.
- `view` calls use CALL, not STATICCALL (a Solang choice; a write in the
  callee is not prevented).
- A frame's heap allocations leak until the end of the transaction.
- Depth is limited to 1024 frames, but the guest's stack might run out
  before that; untested.
