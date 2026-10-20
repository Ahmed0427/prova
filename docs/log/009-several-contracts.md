# 009: several contracts in one guest

## 1. What and why

Calls between contracts (010) need several contracts in one program, and a
way to find the code at an address. Until now the guest linked exactly one
`build/contract.o` and the world was one storage map. Now `./prova compile
A.sol B.sol` links every contract of the files; the guest keeps an address
→ contract registry and per-account storage; the input and public values
carry the state as a list of accounts; a deploy step names the contract.

## 2. Decision

**(a) Unique symbols per contract.** Every object exports `solang_deploy`
and `solang_call`, and also defined `__ashlti3`, `__ashrti3`, `__lshrti3`
(LLVM calls these by name, so they can't be internal).

| Option | For | Against |
|---|---|---|
| A. The compiler names the entry points per contract (`solang_call_erc20`) | no extra tool | the object's interface then depends on the contract name; every runtime, including a one-contract r55-style VM, has to know the scheme; two contracts named alike in different files still collide |
| B. A compiler flag for a symbol prefix | the runtime chooses | a new CLI flag on a shared compiler for one runtime's linking needs |
| **C. The object keeps the fixed names; the runtime renames when it links several (`llvm-objcopy --redefine-sym`)** | the interface in the spec stays one fixed pair of names; renaming is a link-time concern of whoever links | needs `llvm-objcopy` (prova already needs the system LLVM for `llvm-objdump`) |

Chosen: **C**, plus one compiler change that every option needs: the three
shift helpers become **weak** (`WeakODR`) instead of global, so identical
copies from several objects link without "duplicate symbol". Making them
internal doesn't work (LLVM's libcalls reference the global name), and
leaving them undefined would make the object import non-`__sys_` symbols.

**(b) What identifies code in the state.** An account stores the
contract's *name* (`code: String`), resolved against the guest's generated
table. A code hash would be more Ethereum-like, but the guest has no
bytecode to hash: the code is part of the program, and the verifying key
already fixes which object each name runs. Recorded in the spec (§5).

**(c) The state in the public values.** `Vec<Account { address, code,
storage }>` sorted, replacing the single storage list. Same reasoning as
for logs (006): full and transparent now, commitments later.

**(d) Deploy collisions and calls to empty addresses** follow Ethereum:
deploying over existing code reverts; a call to an address without code
succeeds with empty output.

**(e) Step syntax.** `deploy NAME ARGS` when several contracts are linked
(`deploy ARGS` still works when there is one). Test scripts: `storage
ADDRESS SLOT = VALUE` checks another account, and a line of only options
(`--to 0x1c`) sets defaults for the following steps, mirroring options
before the steps on the command line. Each test file links the contracts
of its `contract` lines.

## 3. How it works

```
./prova compile A.sol B.sol
    for each file: solang --target riscv -> out/i/NAME.o
    for each object: llvm-objcopy --redefine-sym solang_deploy=prova_deploy_NAME
                                  --redefine-sym solang_call=prova_call_NAME
                     -> build/contracts/NAME.o ; NAME -> build/contracts.txt

guest/build.rs:  for NAME in contracts.txt: link-arg NAME.o
                 generate: extern { prova_deploy_NAME; prova_call_NAME }
                           static CODES = [Code { name, deploy, call }, ...]
host/build.rs:   PROVA_CONTRACTS = names (to resolve `deploy NAME`)

guest main:
    code    = { account.address -> index of account.code in CODES }
    storage = { (address, slot) -> value }
    a = context.address
    deploy NAME: if a has code -> revert; else code[a] = NAME; CODES[NAME].deploy(args)
    call:        if a has code -> CODES[code[a]].call(calldata); else return ""
    __sys_sload/sstore(slot): storage[(a, slot)]
    finish: state_after = reverted ? state_before : accounts from code + storage
```

## 4. Real code

Solang, `src/emit/riscv/mod.rs`: weak helpers.

```diff
-        // LLVM emits calls to these by name, so they must stay visible.
-        const EXPORTS: [&str; 5] = ["solang_deploy", "solang_call", "__ashlti3", "__lshrti3", "__ashrti3"];
+        const EXPORTS: [&str; 2] = ["solang_deploy", "solang_call"];
+        // LLVM emits calls to these by name, so they must stay visible. Weak,
+        // so that several contracts can be linked into one program.
+        const HELPERS: [&str; 3] = ["__ashlti3", "__lshrti3", "__ashrti3"];
 ...
-            if f.count_basic_blocks() > 0 && !EXPORTS.contains(&name) {
-                f.set_linkage(Linkage::Internal);
+            if f.count_basic_blocks() > 0 {
+                if HELPERS.contains(&name) {
+                    f.set_linkage(Linkage::WeakODR);
+                } else if !EXPORTS.contains(&name) {
+                    f.set_linkage(Linkage::Internal);
+                }
```

Solang, `tests/riscv.rs`: the global symbols are exactly the entry points; the helpers are weak.

```diff
-    assert_eq!(defined, set(&["__ashlti3", "__ashrti3", "__lshrti3", "solang_call", "solang_deploy"]));
+    assert_eq!(defined, set(&["solang_call", "solang_deploy"]));
+#[test]
+fn helpers_are_weak() { ... symbols(&compile_testcase("balances.sol"), 2) ... }
```

prova, `prova`: renaming at link time.

```diff
+            "$objcopy" --redefine-sym "solang_deploy=prova_deploy_$name" \
+                --redefine-sym "solang_call=prova_call_$name" \
+                "$object" "$BUILD/contracts/$name.o"
```

prova, `guest/build.rs`: link all, generate the table.

```rust
    for name in names.lines().filter(|name| !name.is_empty()) {
        let object = build.join(format!("contracts/{name}.o"));
        println!("cargo:rustc-link-arg={}", object.display());
        writeln!(externs, "    fn prova_deploy_{name}(input: *const u8, len: u32);").unwrap();
        writeln!(externs, "    fn prova_call_{name}(input: *const u8, len: u32);").unwrap();
        writeln!(table, "    Code {{ name: {name:?}, deploy: prova_deploy_{name}, call: prova_call_{name} }},").unwrap();
    }
```

prova, `guest/src/main.rs`: choosing the entry point.

```rust
    let entry = match &input.deploy {
        // As on Ethereum, deploying over an existing contract fails.
        Some(_) if code.contains_key(&address) => None,
        Some(name) => {
            let index = code_index(name);
            code.insert(address, index);
            Some(CODES[index].deploy)
        }
        None => code.get(&address).map(|index| CODES[*index].call),
    };
```

prova, `common/src/lib.rs`: accounts.

```diff
+pub struct Account {
+    pub address: Address,
+    /// The contract's name, as linked into the guest (`build/contracts.txt`).
+    pub code: String,
+    pub storage: Storage,
+}
 pub struct Input {
     pub context: Context,
-    pub deploy: bool,
+    pub deploy: Option<String>,
     pub calldata: Vec<u8>,
-    pub storage: Storage,
+    pub state: State,
```

## 5. Verification

```
$ ./prova compile contracts/erc20.sol && llvm-nm build/out/0/erc20.o | grep -v ' [tdbrn] '
0000000000000470 W __ashlti3       (weak)
0000000000000470 W __ashrti3 ...
                 U __sys_caller ... U __sys_sstore
0000000000006c1e T solang_call
0000000000006bdc T solang_deploy

$ ./prova test tests/registry.test         (erc20 + incrementer in one guest)
  ok    --from 0xa11ce --to 0xe20 deploy erc20 1000
  ok    --to 0x1c deploy incrementer 5          (storage 0xe20 0x0 still 1000)
  ok    inc(uint32) 2 / get() -> 7              (defaults from a `--to 0x1c` line)
  ok    --to 0x2c deploy incrementer 1 ; get() -> 1 ; --to 0x1c get() -> 7
  ok    --to 0x1c deploy erc20 1                reverts (code exists)
  ok    --to 0xdead get()                       returns nothing
tests/registry.test: 11/11 steps passed

$ ./prova run '--to 0x1 deploy 5'        (two contracts linked)
error: deploy which contract? write `deploy NAME ...` with one of: erc20, incrementer

$ ./prova run --from 0xa11ce '--to 0x1c deploy incrementer 5' '--to 0xe20 deploy erc20 7'
  state after     0x1c incrementer {0x0 = 0x5}
                  0xe20 erc20 {0x0 = 0x7, 0x6203…38ec = 0x7}
```

Tests that never deployed now start with `deploy` (a call to an empty
address returns nothing, as on Ethereum); `context.test` deploys a second
instance for its `--to 0x1234 me()` step.

```
$ ./prova test             all 9 test files passed
$ cargo test --test riscv  ok. 5 passed
```

## 6. Commits

- Solang: `a7849c2a make riscv shift helpers weak so several contracts link together`
- prova: `26379da link several contracts into the guest with an address registry`

## 7. Open issues / follow-ups

- Contract creation from a contract (`new C()`) isn't supported; accounts
  are created by top-level deploy steps only.
- Each test file still rebuilds the guest. All tests could share one guest
  with every test contract linked, now that names are explicit.
