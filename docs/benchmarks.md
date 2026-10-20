# Benchmarks: native RISC-V vs revm in SP1

The same transactions, run two ways inside the SP1 zkVM (SP1 6.8, executor
only, no proofs):

- **native**: the contract compiled by Solang (`sp1-target`) to RV64IM and
  linked into the guest (`guest/`);
- **revm**: the same Solidity source compiled by `solc 0.8.36 --optimize`,
  run by revm 38 (`evm-guest/`).

Both guests read the same transaction and commit the same public values;
`./prova bench` checks that they agree on every step (output, revert data,
events, storage) before it reports any number. Both use SP1's keccak and
SHA-256 precompiles (patched `tiny-keccak`/`sha3` and `sha2`).

Numbers are RISC-V cycles from SP1's executor (`ExecutionReport`), which are
deterministic: rerunning gives the same counts. Measured at prova
`448d728`, Solang `c4fc132b`.

## Summary

Cycles per transaction. *Execute* is the contract alone (the native entry
point, or `evm.transact`); *total* adds reading the input and building the
state (setup) and building and committing the public values (commit).

| workload | native execute | revm execute | execute × | native total | revm total | total × |
|---|--:|--:|--:|--:|--:|--:|
| ERC-20 `transfer` | 16,008 | 56,837 | **3.6** | 139,361 | 363,508 | **2.6** |
| ERC-20 `transferFrom` | 24,837 | 74,309 | **3.0** | 209,799 | 429,918 | **2.0** |
| ERC-20 `balanceOf` | 5,058 | 35,392 | **7.0** | 113,120 | 326,735 | **2.9** |
| ERC-20 `transfer`, too little balance (reverts) | 7,427 | 42,319 | **5.7** | 132,152 | 361,713 | **2.7** |
| `fib(300)`: uint256 additions | 17,867 | 634,598 | **35.5** | 59,887 | 852,128 | **14.2** |
| `sort(100)`: bubble sort in memory | 847,395 | 53,722,580 | **63.4** | 889,415 | 53,940,110 | **60.6** |
| `primes(1000)`: uint256 `%` and `*` | 6,670,481 | 19,259,019 | **2.9** | 6,712,501 | 19,476,549 | **2.9** |
| `hashChain(100)`: keccak in a loop | 173,428 | 625,675 | **3.6** | 215,448 | 843,299 | **3.9** |
| A calls B, B reverts, A catches (`tryRevert`) | 25,266 | 103,019 | **4.1** | 172,878 | 511,454 | **3.0** |

## Where the cycles go

One ERC-20 `transfer` (`tests/erc20.test`, step 6):

| | setup | execute | commit | outside the regions | total |
|---|--:|--:|--:|--:|--:|
| native | 13,032 | 16,008 | 107,979 | 2,342 | 139,361 |
| revm | 194,028 | 56,837 | 99,363 | 13,280 | 363,508 |

- **Execution** is 3-4× cheaper natively for storage-bound ERC-20 calls and
  up to 35-63× for arithmetic- and memory-bound code. A storage access costs
  about the same on both sides (a map lookup in the guest, and a keccak
  precompile call for each mapping key), so the ERC-20 ratio is set by the
  code around them: dispatch, ABI decoding, checks.
- **Setup** is ~13-23k natively against ~165-270k for revm, which builds
  its EVM context and hashes every account's code (the 9-26 keccak-f
  permutations revm shows on steps with no keccak in the contract).
- **Commit** is the same order for both (it builds and hashes the same
  public values) and dominates the native total: the full state is
  serialized twice (before and after), and serde serializes `[u8; 32]`
  element by element. It grows with the state, which is why the ERC-20
  steps get more expensive along the script. Committing hashes instead of
  full state would cut it for both (log 013, option C).
- **`primes`** is the outlier: 256-bit `%` and `/` on the native side go
  through Solang's `udivmod256` (`stdlib/bigint.c`), a shift-and-subtract
  long division with one loop iteration per quotient bit, each on 256-bit
  values (several RV64 instructions and 128-bit shift helper calls per
  step). revm's `ruint` divides word by word. A 64-bit fast path when both
  operands fit, or a better 256-bit division, is the clearest compiler-side
  win.
- **keccak** costs the same per permutation on both sides (the
  precompile); `hashChain`'s ratio comes from the loop and `abi.encode`
  around it.

## All steps

ERC-20 (`tests/erc20.test`):

| step | native execute | native total | revm execute | revm total | execute ×  | total × | keccak-f (native / revm) |
|---|--:|--:|--:|--:|--:|--:|--:|
| `--from 0xa11ce deploy 1000` | 8855 | 83467 | 43542 | 320968 | 4.9 | 3.8 | 1 / 15 |
| `name()` | 2833 | 93609 | 32912 | 310890 | 11.6 | 3.3 | 0 / 12 |
| `symbol()` | 2775 | 93551 | 33206 | 311184 | 12.0 | 3.3 | 0 / 12 |
| `decimals()` | 2386 | 84193 | 27835 | 296214 | 11.7 | 3.5 | 0 / 12 |
| `totalSupply()` | 3270 | 85077 | 30685 | 299471 | 9.4 | 3.5 | 0 / 12 |
| `--from 0xa11ce transfer(address,uint256) 0xb0b 300` | 16008 | 139361 | 56837 | 363508 | 3.6 | 2.6 | 4 / 15 |
| `balanceOf(address) 0xa11ce` | 5058 | 113120 | 35392 | 326735 | 7.0 | 2.9 | 1 / 13 |
| `balanceOf(address) 0xb0b` | 5230 | 113292 | 35392 | 326735 | 6.8 | 2.9 | 1 / 13 |
| `balanceOf(address) 0xca7` | 4750 | 112812 | 36674 | 334698 | 7.7 | 3.0 | 1 / 13 |
| `--from 0xb0b transfer(address,uint256) 0xca7 301` | 7427 | 132152 | 42319 | 361713 | 5.7 | 2.7 | 1 / 13 |
| `--from 0xb0b transfer(address,uint256) 0xca7 300` | 16752 | 161609 | 56858 | 390374 | 3.4 | 2.4 | 4 / 15 |
| `balanceOf(address) 0xb0b` | 5402 | 135231 | 35392 | 343818 | 6.6 | 2.5 | 1 / 13 |
| `balanceOf(address) 0xca7` | 5058 | 134887 | 35392 | 343818 | 7.0 | 2.5 | 1 / 13 |
| `--from 0xca7 transfer(address,uint256) 0xca7 100` | 16175 | 173370 | 55743 | 388975 | 3.4 | 2.2 | 4 / 15 |
| `balanceOf(address) 0xca7` | 5058 | 134887 | 35392 | 343818 | 7.0 | 2.5 | 1 / 13 |
| `--from 0xa11ce transfer(address,uint256) 0 1` | 4949 | 151086 | 36105 | 355214 | 7.3 | 2.4 | 0 / 12 |
| `--from 0xa11ce --value 1 transfer(address,uint256) 0xb0b 1` | 1815 | 129141 | 26947 | 325455 | 14.8 | 2.5 | 0 / 12 |
| `--from 0xa11ce approve(address,uint256) 0xb0b 50` | 10856 | 178183 | 44498 | 385698 | 4.1 | 2.2 | 2 / 14 |
| `allowance(address,address) 0xa11ce 0xb0b` | 6604 | 164260 | 40682 | 371839 | 6.2 | 2.3 | 2 / 14 |
| `--from 0xb0b transferFrom(address,address,uint256) 0xa11ce 0xca7 20` | 24837 | 209799 | 74309 | 429918 | 3.0 | 2.0 | 8 / 19 |
| `allowance(address,address) 0xa11ce 0xb0b` | 6604 | 164260 | 40682 | 371839 | 6.2 | 2.3 | 2 / 14 |
| `--from 0xb0b transferFrom(address,address,uint256) 0xa11ce 0xca7 31` | 8495 | 176567 | 45800 | 380870 | 5.4 | 2.2 | 2 / 14 |
| `--from 0xca7 transferFrom(address,address,uint256) 0xa11ce 0xca7 1` | 8702 | 176774 | 45866 | 380936 | 5.3 | 2.2 | 2 / 14 |
| `--from 0xa11ce approve(address,uint256) 0xb0b 0xffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff` | 10192 | 189944 | 44450 | 394929 | 4.4 | 2.1 | 2 / 14 |
| `--from 0xb0b transferFrom(address,address,uint256) 0xa11ce 0xb0b 80` | 21767 | 206729 | 66553 | 422162 | 3.1 | 2.0 | 6 / 17 |
| `allowance(address,address) 0xa11ce 0xb0b` | 6604 | 164260 | 40682 | 371839 | 6.2 | 2.3 | 2 / 14 |
| `--from 0xa11ce approve(address,uint256) 0 1` | 4717 | 172347 | 35316 | 370102 | 7.5 | 2.1 | 0 / 12 |
| `balanceOf(address) 0xa11ce` | 5402 | 157788 | 35392 | 360933 | 6.6 | 2.3 | 1 / 13 |
| `balanceOf(address) 0xb0b` | 5574 | 157960 | 35392 | 360933 | 6.3 | 2.3 | 1 / 13 |
| `balanceOf(address) 0xca7` | 5230 | 157616 | 35392 | 360933 | 6.8 | 2.3 | 1 / 13 |
| `totalSupply()` | 3270 | 150905 | 30765 | 351615 | 9.4 | 2.3 | 0 / 12 |

Compute (`tests/compute.test`):

| step | native execute | native total | revm execute | revm total | execute ×  | total × | keccak-f (native / revm) |
|---|--:|--:|--:|--:|--:|--:|--:|
| `deploy` | 1776 | 28010 | 26676 | 235976 | 15.0 | 8.4 | 0 / 9 |
| `fib(uint256) 50` | 5021 | 47041 | 130848 | 348378 | 26.1 | 7.4 | 0 / 9 |
| `fib(uint256) 300` | 17867 | 59887 | 634598 | 852128 | 35.5 | 14.2 | 0 / 9 |
| `primes(uint256) 100` | 277366 | 319386 | 937881 | 1155411 | 3.4 | 3.6 | 0 / 9 |
| `primes(uint256) 1000` | 6670481 | 6712501 | 19259019 | 19476549 | 2.9 | 2.9 | 0 / 9 |
| `sort(uint256) 20` | 85715 | 127735 | 2112924 | 2330454 | 24.7 | 18.2 | 0 / 9 |
| `sort(uint256) 1` | 4822 | 46842 | 44978 | 262508 | 9.3 | 5.6 | 0 / 9 |
| `sort(uint256) 100` | 847395 | 889415 | 53722580 | 53940110 | 63.4 | 60.6 | 0 / 9 |
| `hashChain(uint256) 3` | 7073 | 49093 | 47328 | 264858 | 6.7 | 5.4 | 3 / 12 |
| `hashChain(uint256) 100` | 173428 | 215448 | 625675 | 843299 | 3.6 | 3.9 | 100 / 109 |

Calls between contracts (`tests/calls.test`; revm runs the same calls
between two EVM contracts):

| step | native execute | native total | revm execute | revm total | execute ×  | total × | keccak-f (native / revm) |
|---|--:|--:|--:|--:|--:|--:|--:|
| `--to 0xbbbb deploy callee` | 1765 | 27960 | 26027 | 291473 | 14.7 | 10.4 | 0 / 6 |
| `--to 0xaaaa deploy caller 0xbbbb` | 3665 | 58226 | 40095 | 384665 | 10.9 | 6.6 | 0 / 26 |
| `callSet(uint256) 7` | 13694 | 123418 | 68802 | 443800 | 5.0 | 3.6 | 0 / 25 |
| `tryRevert(uint256) 9` | 25266 | 172878 | 103019 | 511454 | 4.1 | 3.0 | 0 / 25 |
| `tryRevert(uint256) 3` | 18888 | 157581 | 82234 | 481817 | 4.4 | 3.1 | 0 / 25 |
| `rawRevert(uint256) 9` | 17486 | 165758 | 80881 | 491035 | 4.6 | 3.0 | 0 / 25 |
| `uncaught(uint256) 9` | 18004 | 144239 | 71122 | 451654 | 4.0 | 3.1 | 0 / 25 |
| `who()` | 9351 | 129814 | 57341 | 438104 | 6.1 | 3.4 | 0 / 25 |
| `reenter()` | 20187 | 131394 | 77445 | 447719 | 3.8 | 3.4 | 0 / 25 |
| `staticRead()` | 8282 | 137762 | 63271 | 453895 | 7.6 | 3.3 | 0 / 25 |
| `staticWrite(uint256) 1` | 6256 | 127206 | 55235 | 434982 | 8.8 | 3.4 | 0 / 25 |
| `delegateSet(uint256) 0xdead` | 10386 | 144781 | 65239 | 458491 | 6.3 | 3.2 | 0 / 25 |

## Reproduce

Requirements: everything `./prova setup` needs, plus `solc` (0.8.x) on PATH.

```bash
./prova setup
./prova bench tests/erc20.test      # tables go to build/bench/erc20.md
./prova bench tests/compute.test
./prova bench tests/calls.test
```

`./prova bench FILE` compiles the file's contracts with Solang (and the
file's `solang` flags), compiles the same sources with `solc --optimize
--bin --bin-runtime`, then runs every step on both guests with the trace
off, compares their public values, and prints the table. It exits non-zero
if any step disagrees.

## Caveats

- **Cycles, not proving time.** Proving cost is roughly proportional to
  cycles, but precompile calls, memory and the number of shards have their
  own costs. No proof was generated (not enough RAM on this machine).
- **Same state model, not Ethereum's.** Neither guest proves its starting
  state (no state root), and the EVM guest is given its code as input,
  without a code-hash check. Both commit full state; a production system
  would commit roots. The benchmark compares execution under these equal
  conditions.
- **Gas is off.** revm runs without gas limits or fee charging, and the
  native side has no gas metering at all. Native contracts metered by
  instrumentation would cost more (see `docs/design-legacy-evm-and-gas.md`).
- **revm configuration.** revm 38 with `default-features = false`, the
  `optional_*` features that turn off balance, base fee and gas-limit
  checks, the latest hard fork (`Context::mainnet()`), and SP1's `sha3`
  and `sha2` patches. Code hashing at account load is part of its setup.
- **Compiler settings.** solc `--optimize` (200 runs, legacy pipeline, no
  `--via-ir`). Solang with LLVM -O2. `compute.sol` is compiled with
  `--no-cse` because Solang's CSE pass miscompiles its `sort` on every
  target (log 012); for the ERC-20, `--no-cse` changes no measured
  execution by more than 2 cycles.
- **Storage layout.** The ERC-20 has the same storage layout under both
  compilers (all values are full words or mappings). Contracts with small
  adjacent state variables don't: solc packs them, this target doesn't
  (log 004), so a native contract would do fewer read-modify-write
  operations than solc's code there.
- **What revm does that native code doesn't:** gas accounting (even when
  not charged), the EVM's memory expansion and stack checks, jump
  destination analysis, and precompile dispatch on every call. That is the
  interpreter overhead this comparison is meant to show, not noise.
