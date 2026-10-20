# 015: design note: legacy EVM calls and gas (`docs/design-legacy-evm-and-gas.md`)

## 1. What and why

Phase 4 asks for a design, no code: how a native contract could call a
legacy EVM contract through revm in the same proof, and how to meter gas
for native code. The note gives the options and a recommendation for each,
grounded in what phases 2 and 3 built and measured.

## 2. Decision

**Legacy EVM calls.** Options: (A) separate proofs linked by state roots;
(B) one guest, the native runtime in charge, revm called per EVM call over
a `Database` adapter; (C) one guest, **revm in charge of state and the call
stack, native contracts as a second frame type**. Recommended: **C**. B
needs two journals kept consistent through an inspector (every revert rule
twice); A can't do synchronous native → EVM → native calls without
exchanging state at every crossing.

The point that shapes C, checked in revm 38's source: its frame loop
(`Handler::run_exec_loop`) is iterative and can't be re-entered from inside
a frame (`frame_return_result` returns only when the frame stack is empty;
a `PrecompileProvider` gets only the context). So a native frame must be a
**coroutine on its own stack** that yields its `__sys_call` to revm's loop.
That is the two-way version of the register save/restore the guest already
does for frames (log 010). Native accounts are marked by code with an
`0xEF` prefix, which EIP-3541 keeps out of EVM code.

**Gas.** Options: (1) none, price by SP1 cycles afterwards; (2) read SP1
cycles at run time (no such syscall); (3) **compiler-inserted counters per
basic block**, in a unit proportional to RISC-V instructions, with the
counter as a runtime-owned data import `__sys_gas`; (4) rewriting the
object; (5) counters on back-edges and calls only. Recommended: **3**,
starting with its cheapest form (loop headers and function entries), with
SP1 cycles kept as the pricing check. Only 3 gives in-proof out-of-gas,
`gasleft()` and 63/64 forwarding to EVM callees. Estimated overhead
+15-30% of native execution, from the benchmark's ERC-20 numbers.

## 3. How it works

See the note; the core of C:

```
enum Frame { Evm(EthFrame), Native(NativeFrame) }
frame_init:  native marker code -> Native (own stack), else Evm
frame_run(Native): resume until __sys_return/__sys_revert (-> result)
                   or __sys_call (-> yield FrameInput::Call to revm's loop)
native __sys_sload/sstore/log -> revm journal; checkpoints are revm's
```

## 4. Real code

None (design only).

## 5. Verification

The revm facts the design depends on were read from the revm 38 sources
in the cargo registry: `revm-handler-18.1.0/src/handler.rs`
(`run_exec_loop`, `frame_return_result`), `precompile_provider.rs`
(`run(&mut self, context: &mut CTX, inputs: &CallInputs)`),
`revm-context-16.0.1/src/journal.rs` (`checkpoint`, `checkpoint_revert`).
Cost figures are from `docs/benchmarks.md`.

## 6. Commits

- Solang: none.
- prova: `74048a3 add a design note for legacy evm calls and gas metering`

## 7. Open issues / follow-ups

- The overhead estimate for gas counters is an estimate; measure it with
  `./prova bench` once implemented.
- Native stack size per coroutine frame needs a number (measure the
  deepest native frame in the test contracts).
