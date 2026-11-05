# Proving on the Succinct prover network

`./prova prove` proves the last step on your own machine by default. That
needs a lot of memory and computing power. With `--network`, the proof is made
by the **Succinct prover network** instead, and you pay for it in the PROVE
token. Everything else works the same: the same steps, output and checks.

Where each fact comes from:

- **[docs]**: docs.succinct.xyz: the prover-network
  [quickstart](https://docs.succinct.xyz/docs/sp1/prover-network/quickstart),
  [advanced usage](https://docs.succinct.xyz/docs/sp1/prover-network/advanced-usage),
  [troubleshooting](https://docs.succinct.xyz/docs/sp1/prover-network/troubleshooting)
  and [proof types](https://docs.succinct.xyz/docs/sp1/generating-proofs/proof-types).
- **[sdk]**: the source of `sp1-sdk` 6.8.0, the version prova uses.
- **unconfirmed**: we have not checked it, or we could not find it in either
  source.

**Status: no real network proof has been made with prova yet.** The code
compiles and `run`/`test`/`bench` still pass, but nothing on this page has
been tested end to end.

## What happens

1. **Execute locally.** Every step except the last runs on your machine, as
   with `./prova run`. This is cheap.
2. **Set up locally.** prova computes the program's verifying key ("vkey") on
   your machine. [sdk] (How much memory this needs: unconfirmed, but it is
   not proving.)
3. **Simulate locally.** Before sending the request, the SDK runs the program
   once on your machine. [docs, sdk]
4. **Prove remotely.** The SDK sends the program and its input to the network.
   Provers bid in an auction, and the winner makes the proof. [docs] prova
   asks for a **compressed** proof: a STARK proof of constant size, not meant
   for on-chain verification. [docs]
5. **Verify locally.** prova checks the proof on your machine. Then it changes
   one public value (`reverted`) and checks that the changed proof is
   **rejected**. This shows the proof is bound to all its public values
   (inputs and outputs), not just `reverted`. It is the same check as the
   local path.

The proven step runs without the per-syscall trace. The trace only adds
cycles to prove, and it is not part of the public values.

The proof is saved to `build/proof.bin` before the tamper check.

## What you need

1. **A requester account.** This is an Ethereum-style key pair (secp256k1).
   You can make one with Foundry (`cast wallet new`) or MetaMask. [docs] Use
   a new key that holds only what you need for proving.
2. **Its private key in `NETWORK_PRIVATE_KEY`.** It should start with `0x`,
   followed by 64 hex characters. [docs] prova stops with an error before any
   work if the key is missing or empty. prova never prints or stores the key.
3. **PROVE deposited into the network.** You get PROVE on Ethereum mainnet.
   Then connect the wallet to the
   [Succinct Explorer](https://explorer.succinct.xyz/account), click
   **Deposit** on the Account page, and confirm in your wallet. Indexing can
   take up to 5 minutes, after which the balance shows on the Account page.
   [docs]
4. **A little ETH for gas on Ethereum mainnet**, for buying PROVE and for the
   deposit transaction. *Unconfirmed:* the quickstart does not mention ETH.
   It does say the deposit asks you to sign a permit and a transaction.
5. **The SP1 toolchain and a working `./prova run`**, as for every other
   command.

Environment variables:

| variable              | needed? | meaning |
|-----------------------|---------|---------|
| `NETWORK_PRIVATE_KEY` | yes     | requester private key [docs, sdk] |
| `SP1_PROVER=network`  | no      | the same as `--network` (only `prove` reads it) |
| `NETWORK_RPC_URL`     | no      | network RPC. The SDK's default for mainnet is `https://rpc.mainnet.succinct.xyz`. [sdk] |
| `RUST_LOG`            | no      | prova sets it to `sp1_sdk=info` if it is unset, so that you see the request id and explorer link |

Keep the key out of the repository. `.gitignore` ignores `.env`, `.env.*` and
`*.env`, but the safest way is to type the key into your shell:

```bash
read -rs NETWORK_PRIVATE_KEY && export NETWORK_PRIVATE_KEY
```

## First real run

Run these in order, and only go on when one succeeds. Each step's proof
replaces `build/proof.bin`.

```bash
# 0. Check that everything works locally (free).
./prova compile contracts/incrementer.sol
./prova run 'deploy 5' 'inc(uint32) 5' 'get()'

# 1. Prove the smallest step: a deploy.
./prova prove --network 'deploy 5'

# 2. Prove a call. The deploy runs locally, and only inc is proven.
./prova prove --network 'deploy 5' 'inc(uint32) 5'

# 3. Prove an ERC-20 transfer.
./prova compile contracts/erc20.sol
./prova prove --network '--from 0xa11ce deploy 1000' \
    '--from 0xa11ce transfer(address,uint256) 0xb0b 300'
```

`SP1_PROVER=network ./prova prove 'deploy 5'` does the same as step 1.

## Expected output

This is roughly what to expect. The `├─` lines come from the SDK. [sdk]

```
== proving deploy 5
  setup           ...
  vkey            0x...
 INFO Requesting proof:
 INFO ├─ Strategy: Auction
 INFO ├─ Proof mode: Compressed
 INFO ├─ Circuit version: ...
 INFO ├─ Timeout: 3600 seconds
 INFO ├─ Base fee: ... (... $PROVE)
 INFO ├─ Max price per bPGU: ... (... $PROVE)
 INFO ├─ Cycle limit: ... cycles
 INFO └─ Gas limit: ... PGUs
 INFO Created request 0x... in transaction ...
 INFO View request status at: https://explorer.succinct.xyz/request/0x...
 INFO Proof request assigned, proving...
  proved          ...
  proof           .../build/proof.bin, ... bytes
  state before    {}
  reverted        false
  output          0x...
  state after     0xc0de incrementer {0x0 = 0x5}

  verify(proof)                      accepted
  verify(proof, reverted flipped)    rejected
```

**It worked if** the last two lines say `accepted` and `rejected`, and the
command exits with status 0 (`echo $?`). If either line is wrong, the exit
status is 1. The request is also listed on the explorer link.

## Costs

What is known:

- You pay in PROVE, from the requester account's balance on the network.
  [docs]
- The price is set by an **auction**. The work is measured in "prover gas
  units" (PGUs). [docs]
- Every request has a **base fee**, plus a **max price per PGU**. Both are
  shown in the "Requesting proof" log before the proof starts. [sdk]
- The SDK takes the base fee and the market max price from the network.
  Unless you set it yourself, it adds a 20% buffer to the max price. [docs, sdk]
  prova does not set a price.
- The quickstart's *example* log shows a base fee of 0.2 PROVE and a max
  price of 2 PROVE per billion PGUs. [docs] Those are the numbers for their
  example program, not a price list.
- If a request is not fulfilled before the timeout, it is aborted **and fees
  are not refunded**. [docs] prova's timeout is 1 hour (`NETWORK_TIMEOUT` in
  `host/src/main.rs`).

Unconfirmed:

- What one prova proof costs, in PROVE or in money.
- How the PGUs for a step relate to its cycle count. `./prova run` prints
  cycles: `./prova run 'deploy 5'` reports about 122k cycles for the
  incrementer, with the trace on. The proven step runs without the trace, so
  it should be well below `./prova run`'s numbers.
- The minimum deposit, and the ETH gas for buying and depositing PROVE.

## Risks

- **The inputs are visible to the prover.** The step's input (calldata,
  context and the whole starting storage) goes to the network. The docs say
  inputs are hidden from bidders only with "private stdin", which is a gated
  feature. [docs] prova does not use it. So assume the winning prover, and
  possibly others, can read the input. Do not prove steps with secret data.
- **SDK version support is unconfirmed.** prova uses sp1-sdk 6.8.0. The
  quickstart says "sp1-sdk >= v6.1.0". [docs] We have not confirmed that the
  mainnet network accepts 6.8.0 today. Verification fails if the proof's SP1
  version differs from the local one. [sdk]
- **Key safety.** Whoever has the key can spend the PROVE deposited for it.
  Use a dedicated key with a small balance. Never commit it, and never paste
  it into logs, issues or chats.
- **Paid failures.** A timed-out request is not refunded. [docs] Start with
  the smallest step (`deploy 5`).
- **Memory of the local parts** (setup, simulation and verification):
  unconfirmed, but they need far less than proving.

## Troubleshooting

| what you see | what to do |
|---|---|
| `error: network proving needs NETWORK_PRIVATE_KEY ...` (from prova) | `export NETWORK_PRIVATE_KEY=0x...` in the same shell. |
| `Failed to create local signer` (SDK panic) | The key is malformed: it should be `0x` and 64 hex characters. [docs] |
| `Insufficient funds for proof request` | Check the balance on the [Account page](https://explorer.succinct.xyz/account), deposit more PROVE, and wait up to 5 minutes for indexing. [docs] |
| `Proof request 0x... timed out during the auction` | No prover bid. Try again later. A higher max price may help (prova does not expose it yet). [sdk] |
| `Proof request 0x... timed out` | Not proven within 1 hour. The fee is not refunded. [docs] Check the request on the explorer. |
| `Proof request 0x... is unexecutable` | The program failed to run. First check that the same steps work with `./prova run`. [docs, sdk] |
| `Failed to connect to RPC endpoint` | Check your connection, and `NETWORK_RPC_URL` if you set it. [docs] |
| `verify(proof)  rejected` | Most likely an SP1 version mismatch between the network and sp1-sdk 6.8.0. prova only prints accepted/rejected, so the reason is not shown. Compare the "Circuit version" log line with the explorer. [sdk] |
| `Failed to register program` | Rebuild (`cargo clean`, then build again) and retry. [docs] |
