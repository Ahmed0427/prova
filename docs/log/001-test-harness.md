# 001: test harness (`./prova test`)

## 1. What and why

Until now the only check of runtime behaviour was reading `./prova run`
output by eye. Every later change (keccak, mappings, events, nested calls)
changes the guest and the compiler together, so it needs a regression suite
that runs the real pipeline: Solang → `contract.o` → SP1 guest → executor.
`./prova test` compiles each test file's contract, runs its steps in SP1, and
checks the results it states.

## 2. Decision

How to write tests:

| Option | For | Against |
|---|---|---|
| A. Shell scripts that grep `./prova run` output | no new code | brittle (depends on print format), poor failure messages |
| B. `cargo test` integration tests in `host/` | standard Rust tooling | `include_elf!` embeds one guest per host build, so one contract per test binary; the compile step lives outside cargo; tests become Rust code instead of transaction lists |
| **C. Declarative `tests/*.test` scripts, checked by the host** | reads like a transcript; same step syntax as `./prova run`; failure shows file:line and the guest trace | a tiny custom format to maintain |

Chosen: **C**. The format is line-based: unindented lines are steps, indented
lines under a step are expectations. A step without `reverts` must succeed,
so an unexpected revert is never silently accepted. Expectations are only
what the public values contain (`reverted`, `output`, `storage_after`), so
the test checks exactly what a proof would attest to. More kinds (`log`,
context like `from`) will be added with the features that need them.

Cost, accepted for now: each test file links a different `contract.o`, so the
guest is rebuilt per file (~30 s each). Phase 2 links several contracts into
one guest, which can bring this down.

## 3. How it works

```
./prova test [FILE...]                      (bash)
  for file in FILES or tests/*.test:
    contract = line "contract FILE.sol [NAME]"
    ./prova compile contract                -> build/contract.o
    cargo run -p prova-host -- --test file  (rebuilds the guest if contract.o changed)
  exit 1 if any file failed

prova-host --test FILE                      (host/src/main.rs, script.rs)
  cases = parse(FILE)                       # step line + indented expectations
  storage = {}
  for case in cases:
    public, trace, cycles = run(step, storage)   # SP1 executor, no proof
    failures = check(case, public)
      reverted != case.reverts          -> failure
      output   != each Output expect    -> failure
      storage_after[slot] != value      -> failure (missing slot = 0)
    print "ok ... cycles"  or  "FAIL", failures, calldata, guest trace, public values
    storage = public.storage_after
```

## 4. Real code

prova, `host/src/script.rs` (new): expectations and the check.

```rust
pub enum Expect {
    /// The exact output bytes: `returns WORD...` or `output 0xHEX`.
    Output(Vec<u8>),
    /// `storage SLOT = VALUE` after the step; 0 also matches a missing slot.
    Storage(Word, Word),
}
...
pub fn check(case: &Case, public: &Public) -> Vec<String> {
    let mut failures = Vec::new();
    if public.reverted != case.reverts {
        failures.push(format!("expected {}, but it {}", ...));
    }
    for expect in &case.expect {
        match expect {
            Expect::Output(output) if *output != public.output => failures.push(...),
            Expect::Storage(slot, value) => {
                let actual = public.storage_after.iter()
                    .find(|(s, _)| s == slot).map_or([0; 32], |(_, v)| *v);
                if actual != *value { failures.push(...) }
            }
            _ => {}
        }
    }
    failures
}
```

prova, `host/src/main.rs`: step parsing returns errors instead of exiting, so
a test file can report the line.

```diff
-fn parse_step(text: &str) -> Step {
+fn parse_step(text: &str) -> Result<Step, String> {
     let mut parts = text.split_whitespace();
-    let head = parts.next().unwrap_or_else(|| fail("empty step"));
+    let head = parts.next().ok_or("empty step")?;
```

prova, `host/src/main.rs`: execution split from printing; the test loop.

```diff
+fn run(prover: &CpuProver, step: &Step, storage: &Storage) -> (Public, String, u64) {
     let (guest_stdout, log) = tokio::sync::watch::channel(String::new());
     let (values, report) = prover
         .execute(ELF, stdin(step, storage))
...
+fn test(prover: &CpuProver, path: &str) -> bool {
+    ...
+    for case in &cases {
+        let (public, trace, cycles) = run(prover, &case.step, &storage);
+        let failures = script::check(case, &public);
+        ...
+        storage = public.storage_after;
+    }
```

prova, `prova`: the `test` command.

```diff
+cmd_test() {
+    local files=("$@") failed=() file contract
+    [[ ${#files[@]} -gt 0 ]] || files=("$ROOT"/tests/*.test)
+
+    for file in "${files[@]}"; do
+        file="$(realpath "$file")"
+        read -ra contract < <(sed -n 's/#.*//; s/^contract[[:space:]]\+//p' "$file")
+        [[ ${#contract[@]} -ge 1 ]] || die "$file has no 'contract FILE.sol [NAME]' line"
+        (cd "$ROOT" && cmd_compile "${contract[@]}") >/dev/null
+        file="$(realpath --relative-to="$ROOT" "$file")"
+        step "test $file"
+        (cd "$ROOT" && cargo run --release -q -p prova-host -- --test "$file") || failed+=("$file")
+    done
```

Tests added: `tests/incrementer.test` (constructor, write, read, unknown
selector, non-canonical `uint32`, short calldata) and
`tests/storage_types.test` (every value type's storage word and ABI word, and
the five non-canonical-input reverts).

## 5. Verification

```
$ ./prova test
==> test tests/incrementer.test
  ok    deploy 5                                              105148 cycles
  ok    inc(uint32) 5                                         187888 cycles
  ok    get()                                                 156440 cycles
  ok    nope()                                                 38095 cycles
  ok    inc(uint32) 0x100000000                                47096 cycles
  ok    inc(uint32)                                            87062 cycles
tests/incrementer.test: 6/6 steps passed
==> test tests/storage_types.test
  ok    deploy                                                580405 cycles
  ...
  ok    clear()                                               735999 cycles
  ok    flag()                                                328271 cycles
tests/storage_types.test: 17/17 steps passed
all 2 test files passed
```

The harness catches failures (a deliberately wrong script, not committed):

```
  FAIL  deploy 5 (.../bad.test:2)
        storage 0x0 is 0x5, expected 0x6
  ...
  FAIL  nope() (.../bad.test:6)
        expected success, but it reverted
  FAIL  inc(uint32) 1 (.../bad.test:7)
        expected a revert, but it succeeded
.../bad.test: 0/4 steps passed
error: failed: .../bad.test                       (exit 1)

expectation without a step:  error: ...: line 2: an expectation needs a step above it
```

Solang is unchanged in this entry. `cargo clippy -p prova-host`: clean.

## 6. Commits

- prova: see `git log` for `add test scripts and ./prova test` (this entry is
  part of that commit, so it cannot contain its own hash; entry 002 records it).

## 7. Open issues / follow-ups

- The cycle counts include the guest's `println!` trace (about 3/4 of the
  cycles in `docs/review.md`'s measurement). Benchmarks (phase 3) must turn
  the trace off; it should become a guest feature or input flag.
- One guest rebuild per test file. Revisit after phase 2.
- The getters in `storage_types` cost ~330k cycles against ~156k for
  `incrementer.get()`; worth a look when benchmarking (likely the trace's
  `format!` of 32-byte words, plus the larger dispatcher).
