# Design note: calling legacy EVM contracts, and gas

Status: design only, nothing here is implemented. It builds on what is:
native contracts talk to the runtime only through `__sys_*` imports
(`docs/spec.md`), and the SP1 guest implements calls between native
contracts with its own frames and storage journal (log 010).

## Recommendations

1. **Legacy EVM contracts in the same proof: put revm in charge of the
   call stack and the state, and run native contracts as a second kind of
   frame inside it.** Native accounts are marked by their code; when revm's
   frame loop reaches one, a *native frame* runs the linked object on its
   own stack, as a coroutine, so that its `__sys_call` can hand the call
   back to revm's loop and resume when the callee is done. The native
   frame's `__sys_sload/sstore/log/...` read and write revm's journal.
   Ethereum semantics at the boundary (revert scope, static calls, depth,
   value, warm/cold, gas forwarding) then come from one place, and the
   contract objects don't change. Keep the phase 2 runtime for
   transactions that touch only native contracts.
2. **Gas: compiler-inserted counters, one charge per basic block, in a
   unit proportional to RISC-V instructions; the runtime charges for
   `__sys_*` calls; SP1's cycle count stays the off-chain price check.**
   It is the only option that gives in-proof out-of-gas reverts and
   `gasleft()`, which the EVM boundary needs (63/64 forwarding). Expected
   cost: about +15-30% on native execution, which keeps native execution
   well below revm's (benchmarks: 16k vs 57k cycles for an ERC-20
   transfer).

The rest of this note is the reasoning.

---

## Part 1: a native contract calling a legacy EVM contract

### What has to hold

- **One proof** covers the native code, the EVM code it calls, and
  anything those call back.
- **Ethereum semantics at the boundary.** A native contract calling an EVM
  contract (and back) must see what it would see between two EVM
  contracts: calldata and return data in the Ethereum ABI (already the
  native ABI), `msg.sender`/`address(this)`/`msg.value` per frame, a revert
  undoing exactly the reverted frame's writes and events, static calls,
  the 1024 depth limit, and gas forwarded by the 63/64 rule.
- **The EVM code is state.** Native code is fixed by the verifying key; EVM
  code is not. It has to be authenticated as state (a code hash under a
  state root, as rsp does), not trusted as input like the benchmark's EVM
  guest does.
- **No contract changes.** Native objects keep the same `__sys_*` imports;
  only the runtime changes.

### Options

**A. Two proofs, linked by state.** Prove the native part and the EVM part
separately (e.g. this guest and rsp), and aggregate them with a recursive
proof that checks the state roots chain up. Simple for independent
transactions, but a single call that crosses the boundary (native → EVM →
native) needs its intermediate state exchanged between proofs at every
crossing. Rejected for synchronous calls.

**B. Both in one guest, the native runtime in charge.** Keep phase 2's
`World` (storage map + journal, frames with resume points). A call from
native code to an EVM account runs a revm transaction-like call over a
`Database` adapter that reads `World`, then writes revm's state diff back
into `World` (journaled, so a later native revert undoes it). Problems:

- EVM → native calls inside that revm call happen while revm holds
  uncommitted state; the native callee must see it, so its
  `__sys_sload/sstore` would have to switch to revm's journal while any
  EVM frame is active. Two journals, with a mode switch.
- revm's inner checkpoints (an EVM sub-call that reverts after a native
  callee succeeded) must also undo the native callee's writes. With two
  journals that needs an `Inspector` mirroring every `call`/`call_end`
  into `World`'s journal. Possible, but every rule ends up implemented
  twice.

**C. Both in one guest, revm in charge (recommended).** One `revm::Evm`
runs every transaction. Native contracts are accounts whose code is a
marker, e.g. `0xEF 0xF1 ‖ u16 index`, pointing into the guest's table of
linked objects. EIP-3541 forbids deploying EVM code that starts with
`0xEF`, so no EVM contract can look like a marker; EOF (`0xEF00`) and
EIP-7702 delegations (`0xEF0100`) use other `0xEF` prefixes. A custom handler gives revm a second frame type:

```
enum Frame { Evm(EthFrame), Native(NativeFrame) }

frame_init(call):
    code = journal.code(call.target)
    if code is a native marker: Native(NativeFrame::new(object, call))   # its own stack
    else:                       Evm(EthFrame::new(...))                  # as revm does now

frame_run(Native(f)):
    resume f's coroutine; it runs until one of:
        __sys_return / __sys_revert  -> FrameResult (output, success/revert, gas left)
        __sys_call(addr, ...)        -> yields FrameInput::Call to revm's loop
frame_return_result(child result) to Native(f):
    stash it as f's return data; the next frame_run resumes f, and
    its __sys_call returns 0/1 to the contract
```

The native `__sys_*` imports become thin wrappers over the active
context: `__sys_sload/sstore` → `journal.sload/sstore(frame.address, …)`,
`__sys_log` → `journal.log`, `__sys_caller/callvalue/address` → the frame's
`CallInputs`, `__sys_keccak256` unchanged. revm's checkpoints
(`journal.checkpoint()` at frame start, `checkpoint_revert` on failure)
now cover native writes too, so the revert rules are revm's.

**Why a coroutine.** revm 38's frame loop (`Handler::run_exec_loop`) is
iterative: a frame returns "call this" (`ItemOrResult::Item`), the loop
runs the child and hands the result back. It cannot be re-entered from
inside a frame: `frame_return_result` returns only when the whole frame
stack is empty, and a precompile hook only receives the context, not the
handler. A native contract, however, issues its call in the middle of
compiled code. The native frame must therefore be suspended at
`__sys_call` and resumed later: that is a coroutine with its own stack.
The guest already does half of this: `prova_enter`/`prova_leave` (log 010)
save and restore `ra`, `sp`, `s0`-`s11` to abandon a frame; a coroutine
switch does the same in both directions between two stacks. Each native
frame needs a stack (e.g. 64-256 KiB from the guest's heap, released when
the frame ends).

What C rules out, and what it costs:

- *Precompile-style hooks only* (register native accounts as precompiles):
  EVM → native works, native → EVM doesn't (a precompile can't call).
- *Recursion instead of coroutines* (build a second `Evm` over the same
  context for each native → EVM call): needs to move the context out of
  the outer `Evm` while it is borrowed; fragile, and every nested call
  grows the native stack with a whole interpreter.
- *Cost.* Native storage access goes through revm's journal (hash maps,
  warm/cold tracking) instead of phase 2's `BTreeMap`; revm's setup is
  ~165-270k cycles per transaction in the benchmark (context, code
  hashing). That is why the phase 2 runtime should stay for transactions
  that never touch an EVM account: the guest can tell from the
  transaction's first call target, and fall over to revm when a native
  frame first calls an EVM account (by then only native frames are live;
  moving their state into a fresh revm journal is one copy).

### Details C has to settle

- **Gas across the boundary.** revm forwards gas by the 63/64 rule and
  charges EVM costs. Native frames need a gas counter for this to mean
  anything (Part 2); with it, a native frame reports gas used in its
  `FrameResult` like an EVM frame.
- **Code authentication.** EVM code comes from state: the guest must check
  `keccak256(code)` against the code hash in an authenticated account
  (state proof against a root, as in rsp). Native code is authenticated by
  the verifying key; its marker in state should name an object by index
  and be checked against the guest's table.
- **Balances.** revm has them; the native runtime (phase 2) doesn't.
  Under C, `msg.value` moves real balances for both kinds of frames.
- **Precompile addresses.** 0x01-0x11 (and 0x100) are precompiles; native
  contracts must not be deployed there (the benchmark ran into this, log
  012).
- **Contract creation.** `CREATE`/`CREATE2` of native contracts would put a
  marker as code; the object must already be linked into the guest.

### A plan

1. Native frame as a coroutine in the phase 2 runtime first (no revm):
   replace `prova_enter`/`prova_leave` with stack switching; tests stay
   green.
2. A revm `Handler`/`EvmTr` with the `Frame` enum; native `__sys_*` over
   revm's journal; `tests/calls.test` with one of the two contracts as
   EVM bytecode (the benchmark harness already compiles both sides).
3. Gas (Part 2), so that native frames can report gas to revm.
4. Authenticated code and state (state roots), then balances.

---

## Part 2: gas metering for native code

### What gas is for here

- **Bounding execution in-proof**: a transaction that runs too long must
  fail with a well-defined result (revert, state undone) inside the proof,
  not by the prover giving up.
- **EVM semantics**: `gasleft()`, gas forwarding to callees (63/64), and
  calls into EVM contracts (Part 1) need a gas number at run time.
- **Pricing**: what the user pays should follow what proving costs.

### Options

**1. No metering; price by SP1 cycles after the fact.** The executor
reports cycles exactly; charge for them. Cheapest (no overhead) and the
fairest proxy for proving cost. But nothing *inside* the program knows the
count: there is no `gasleft()`, no out-of-gas revert, nothing to forward
to an EVM callee. An unbounded loop has to be cut by the prover's cycle
limit, which gives no in-proof result. Fine for pricing, not for
semantics.

**2. SP1 cycles as gas, read at run time.** SP1 has no syscall or CSR that
returns the current cycle count to the guest, and a host-supplied hint
would be unconstrained. Not available.

**3. Compiler-inserted counters per basic block (recommended).** Solang's
emitter (or an LLVM pass in the riscv backend) charges each basic block's
static cost at the block's entry:

```
block:
    __gas -= COST(block)          ; COST from the block's instructions
    if (__gas < 0) __sys_out_of_gas()
    ...
```

- **Where the counter lives.** One counter per transaction, shared by all
  frames, owned by the runtime. Cheapest is a *data* import: the object
  references a 64-bit `__sys_gas` that the runtime defines (still
  "imports only `__sys_*`", but a variable rather than a function), so a
  charge is a load, subtract, store and branch. The runtime sets it at
  frame entry, takes it back at frame exit, and applies call forwarding
  rules.
- **What `COST` counts.** The instructions of the block as `llc` emits
  them would be exact for SP1 cycles but the count is only known after
  instruction selection; counting LLVM IR instructions before codegen is
  an approximation of the same thing, good to within a small factor.
  Either way it is a unit proportional to SP1 cycles ("native gas"),
  not EVM gas.
- **`__sys_*` calls** are charged by the runtime: storage, logs,
  keccak (per 136-byte block: one precompile call), calls (plus what the
  callee uses). Their prices reflect proving cost, not EVM prices.
- **Overhead.** About 4 instructions per charge. The benchmark's ERC-20
  transfer executes in 16k cycles; with blocks averaging 6-10 instructions
  that is +15-30%, so ~19-21k: still about 3× less than revm's 57k.
  Standard reductions apply: one charge for a chain of blocks with a
  single entry, charges hoisted to loop headers with the loop body's
  cost, and no charge in blocks that end in `__sys_revert`.
- **Precedent.** The same scheme meters Wasm contracts (pallet-contracts'
  and wasm-instrument's gas injection, NEAR, CosmWasm).

**4. Rewrite the RISC-V object after compilation.** Insert the counters
into `contract.o` instead of the compiler's IR. Runtime-independent, but
it has to recover basic blocks and fix up relocations in machine code;
fragile. Rejected.

**5. Count backward jumps and calls only.** Cheaper than 3 (a charge per
loop iteration and per call, not per block), enough to bound execution,
but the count no longer tracks cost; pricing would have to fall back to
option 1. A reasonable first step if 3's overhead turns out higher than
estimated.

### Units at the EVM boundary

Native gas measures RISC-V work; EVM gas measures EVM work, and the
benchmark shows the two differ by 3-60× depending on the code. Keep them
apart: a native frame's budget is native gas; a call into EVM code converts
the forwarded budget at a fixed rate (set from benchmarks like
`docs/benchmarks.md`), and converts back what the EVM callee returns. A
user-facing gas limit can be expressed in either; the rate is a parameter
of the chain, not of the contract.

### Recommendation, in order

1. Option 3 with counters on loop headers and function entries first (the
   cheapest form of 3 that still bounds execution), costs from LLVM IR
   instruction counts, `__sys_gas` as a data import, `__sys_out_of_gas`
   ending the frame like a revert that consumes all gas.
2. Measure the overhead with `./prova bench` (the harness compares cycles
   step by step) and refine to full per-block metering if pricing needs
   it.
3. Keep SP1 cycles (option 1) as the check that native gas tracks proving
   cost: across the benchmark workloads, cycles per unit of native gas
   should stay within a narrow band.
