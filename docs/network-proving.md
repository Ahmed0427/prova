# Proving on the Succinct prover network

`./prova prove` proves the last step on your own machine by default. That needs a lot of
memory and computing power. With `--network`, the proof is made by the **Succinct prover
network** instead, and you pay for it in the PROVE token. Everything else works the same:
the same steps, output and checks.

Where each fact comes from:

- **[docs]**: these pages on docs.succinct.xyz:
  - [quickstart](https://docs.succinct.xyz/docs/sp1/prover-network/quickstart)
  - [advanced usage](https://docs.succinct.xyz/docs/sp1/prover-network/advanced-usage)
  - [troubleshooting](https://docs.succinct.xyz/docs/sp1/prover-network/troubleshooting)
  - [proof types](https://docs.succinct.xyz/docs/sp1/generating-proofs/proof-types)
- **[sdk]**: the source of `sp1-sdk` 6.8.0, the version prova uses.
- **unconfirmed**: we have not checked it, or we could not find it in either source.

**Status: no real network proof has been made with prova yet.** The code compiles and
`run`/`test`/`bench` still pass, but nothing on this page has been tested end to end.

## What happens

1. **Execute locally.** Every step except the last runs on your machine.
   - This is the same as `./prova run`, and it is cheap.
2. **Set up locally.** prova computes the program's verifying key ("vkey") here. [sdk]
   - How much memory this needs: unconfirmed, but it is not proving.
3. **Simulate locally.** The SDK runs the program once before sending it. [docs, sdk]
4. **Prove remotely.** The SDK sends the program and its input to the network.
   - Provers bid in an auction, and the winner makes the proof. [docs]
   - prova asks for a **compressed** proof: a STARK proof of constant size. [docs]
   - A compressed proof is not meant for on-chain verification. [docs]
5. **Verify locally.** prova checks the proof on your machine.
   - Then it changes one public value (`reverted`).
   - It checks that the changed proof is **rejected**.
   - This shows the proof is bound to all its public values (inputs and outputs).
   - So it is bound to more than just `reverted`.
   - It is the same check as the local path.

The proven step runs without the per-syscall trace. The trace only adds cycles to prove,
and it is not part of the public values.

The proof is saved to `build/proof.bin` before the tamper check.

## What you need

1. **A requester account**: an Ethereum-style key pair (secp256k1). [docs]
   - You can make one with Foundry (`cast wallet new`) or MetaMask. [docs]
   - Use a new key that holds only what you need for proving.
2. **Its private key in `NETWORK_PRIVATE_KEY`**: `0x`, then 64 hex characters. [docs]
   - prova stops with an error before any work if the key is missing or empty.
   - prova never prints or stores the key.
3. **PROVE deposited into the network.** [docs]
   - You get PROVE on Ethereum mainnet.
   - Connect the wallet to the [Succinct Explorer](https://explorer.succinct.xyz/account).
   - Click **Deposit** on the Account page, and confirm in your wallet.
   - Indexing can take up to 5 minutes. Then the balance shows on the Account page.
4. **A little ETH for gas on Ethereum mainnet**, to buy PROVE and send the deposit.
   - *Unconfirmed:* the quickstart does not mention ETH.
   - It does say the deposit asks you to sign a permit and a transaction.
5. **The SP1 toolchain and a working `./prova run`**, as for every other command.

Environment variables:

- `NETWORK_PRIVATE_KEY` (needed): the requester private key. [docs, sdk]
- `SP1_PROVER=network` (optional): the same as `--network`. Only `prove` reads it.
- `NETWORK_RPC_URL` (optional): the network RPC. [sdk]
  - The SDK's default for mainnet is `https://rpc.mainnet.succinct.xyz`. [sdk]
- `RUST_LOG` (optional): prova sets it to `sp1_sdk=info` if it is unset.
  - That way you see the request id and explorer link.

Keep the key out of the repository. `.gitignore` ignores `.env`, `.env.*` and `*.env`,
but the safest way is to type the key into your shell:

```bash
read -rs NETWORK_PRIVATE_KEY && export NETWORK_PRIVATE_KEY
```

## First real run

Run these in order, and only go on when one succeeds. Each step's proof replaces
`build/proof.bin`.

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

**It worked if** the last two lines say `accepted` and `rejected`, and the command exits
with status 0 (`echo $?`). If either line is wrong, the exit status is 1. The request is
also listed on the explorer link.

## Costs

The auction is just how the network sets prices, and the SDK handles it for you: `./prova
prove --network` sends the request and waits for the proof or an error. There is nothing
to bid or watch. If no prover takes the request, it fails with "timed out during the
auction" and you run the command again.

What is known:

- You pay in PROVE, from the requester account's balance on the network. [docs]
- The price is set by an **auction**. [docs]
- The work is measured in "prover gas units" (PGUs). [docs]
- Every request has a **base fee**, plus a **max price per PGU**. [sdk]
  - Both are shown in the "Requesting proof" log before the proof starts. [sdk]
- **The default max price** has no fixed number in the SDK. [docs, sdk]
  - The SDK asks the network for the current market price per PGU. [docs, sdk]
  - It adds a 20% buffer (`DEFAULT_MAX_PRICE_PER_PGU_BUFFER = 120`). [docs, sdk]
  - So requests still get filled if the price moves a little. [docs, sdk]
  - prova keeps this default.
- **The gas limit** comes from the SDK's local simulation of the step. [sdk]
  - So it matches the step's real work, not a large fixed limit. [sdk]
- **The most you can pay** for one request:
  - It is about the base fee plus the gas limit times the max price per PGU.
  - All three numbers are in the "Requesting proof" log.
  - The winning bid can only be lower.
  - Source: [sdk, unconfirmed: the exact settlement formula]
- The quickstart's *example* log shows these numbers: [docs]
  - a base fee of 0.2 PROVE, and a max price of 2 PROVE per billion PGUs. [docs]
  - Those are the numbers for their example program, not a price list.
- If a request is not fulfilled before the timeout, it is aborted. [docs]
  - Its **fees are not refunded**. [docs]
  - prova's timeout is 1 hour (`NETWORK_TIMEOUT` in `host/src/main.rs`).

Unconfirmed:

- What one prova proof costs, in PROVE or in money.
- How the PGUs for a step relate to its cycle count.
  - `./prova run` prints cycles.
  - `./prova run 'deploy 5'` reports about 122k cycles for the incrementer, trace on.
  - The proven step runs without the trace.
  - So it should be well below `./prova run`'s numbers.
- The minimum deposit, and the ETH gas for buying and depositing PROVE.

## Risks

- **The inputs are visible to the prover.** Do not prove steps with secret data.
  - The step's input (calldata, context, all starting storage) goes to the network.
  - The docs say inputs are hidden from bidders only with "private stdin". [docs]
  - That is a gated feature. [docs] prova does not use it.
  - So assume the winning prover, and possibly others, can read the input.
- **SDK version support is unconfirmed.**
  - prova uses sp1-sdk 6.8.0. The quickstart says "sp1-sdk >= v6.1.0". [docs]
  - We have not confirmed that the mainnet network accepts 6.8.0 today.
  - Verification fails if the proof's SP1 version differs from the local one. [sdk]
- **Key safety.** Whoever has the key can spend the PROVE deposited for it.
  - Use a dedicated key with a small balance.
  - Never commit it, and never paste it into logs, issues or chats.
- **Paid failures.** A timed-out request is not refunded. [docs]
  - Start with the smallest step (`deploy 5`).
- **Memory of the local parts** (setup, simulation and verification): unconfirmed.
  - But they need far less than proving.

## Troubleshooting

What you see, and what to do:

- `error: network proving needs NETWORK_PRIVATE_KEY ...` (from prova)
  - `export NETWORK_PRIVATE_KEY=0x...` in the same shell.
- `Failed to create local signer` (SDK panic)
  - The key is malformed: it should be `0x` and 64 hex characters. [docs]
- `Insufficient funds for proof request`
  - Check the balance on the [Account page](https://explorer.succinct.xyz/account).
  - Deposit more PROVE, and wait up to 5 minutes for indexing. [docs]
- `Proof request 0x... timed out during the auction`
  - No prover bid. Try again later. [sdk]
  - A higher max price may help (prova does not expose it yet). [sdk]
- `Proof request 0x... timed out`
  - Not proven within 1 hour. The fee is not refunded. [docs]
  - Check the request on the explorer.
- `Proof request 0x... is unexecutable`
  - The program failed to run. [docs, sdk]
  - First check that the same steps work with `./prova run`.
- `Failed to connect to RPC endpoint`
  - Check your connection, and `NETWORK_RPC_URL` if you set it. [docs]
- `verify(proof)  rejected`
  - Most likely an SP1 version mismatch between the network and sp1-sdk 6.8.0. [sdk]
  - prova only prints accepted/rejected, so the reason is not shown.
  - Compare the "Circuit version" log line with the explorer. [sdk]
- `Failed to register program`
  - Rebuild (`cargo clean`, then build again) and retry. [docs]
