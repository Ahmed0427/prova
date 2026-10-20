# 013: hash the public values with the SHA-256 precompiles

## 1. What and why

The benchmark (012) showed the *commit* region costing ~47k cycles even for
a contract with no storage. sp1-zkvm hashes the committed public values
with `sha2::Sha256` (`PUBLIC_VALUES_HASHER`), and both guests used the
plain crates.io `sha2`, i.e. SHA-256 in software. Patching `sha2` to SP1's
fork, as Succinct's rsp does, makes it use the `SHA_EXTEND`/`SHA_COMPRESS`
precompiles. Both guests get the same patch, so the comparison stays fair.

## 2. Decision

| Option | For | Against |
|---|---|---|
| A. Leave it | nothing to change | every proof pays software SHA-256 for its public values; inflates "total" for both guests |
| **B. Patch `sha2` in both guests** | standard SP1 practice; one line per guest; exact same effect on both | one more git dependency |
| C. Also shrink what is committed (hashes of state/logs, or a hand-written encoding instead of serde's per-byte `[u8; 32]` serialization) | the larger part of the remaining commit cost | changes the public values format; a separate decision (logs 006, 009) |

Chosen: **B** now; C stays a follow-up.

## 3. How it works

```
guest/Cargo.toml, evm-guest/Cargo.toml:
    [patch.crates-io] sha2 = sp1-patches/RustCrypto-hashes, tag patch-sha2-0.10.9-sp1-6.0.0
sp1_zkvm::io::commit(bytes) -> PUBLIC_VALUES_HASHER.update(bytes)   # now SHA_EXTEND/COMPRESS syscalls
```

## 4. Real code

prova, `guest/Cargo.toml` (and the same line in `evm-guest/Cargo.toml`):

```diff
 [patch.crates-io]
 tiny-keccak = { git = "https://github.com/sp1-patches/tiny-keccak", tag = "patch-2.0.2-sp1-6.2.0" }
+sha2 = { git = "https://github.com/sp1-patches/RustCrypto-hashes", tag = "patch-sha2-0.10.9-sp1-6.0.0", package = "sha2" }
```

## 5. Verification

The lock files now resolve `sha2 0.10.9` to the git patch, and a run shows
the precompiles:

```
$ ./prova run deploy 'fib(uint256) 3'
  precompiles     SHA_EXTEND x4, SHA_COMPRESS x4
```

`./prova bench`, before → after (cycles):

```
                                   native commit       revm commit        native total        revm total
fib(uint256) 300                   47,209 -> 33,057    51,412 -> 37,260    77,577 ->  59,887   870,089 -> 852,128
transfer(address,uint256) … 300   150,435 -> 107,979  142,112 -> 99,363   185,355 -> 139,361   410,066 -> 363,508
```

Both still agree on every step (`compute.test`, `erc20.test`).

```
$ ./prova test        all 11 test files passed
```

## 6. Commits

- Solang: none.
- prova: `448d728 hash public values with the sha-256 precompile in both guests`

## 7. Open issues / follow-ups

- Option C: the remaining commit cost (~33k fixed, growing with the state)
  is mostly serde/bincode serializing `[u8; 32]` arrays element by element,
  twice (state before and after). Committing hashes, or a hand-written
  encoding through `commit_slice`, would cut it.
