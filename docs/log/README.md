# Change log

One file per logical change, in commit order. Each has: what and why, the
decision taken (with the options rejected), how it works, the key hunks of
the real diff, verification, commits, and open issues.

| # | Change | Phase |
|---|---|---|
| [001](001-test-harness.md) | `./prova test`: scripted steps with expected results | 0 |
| [002](002-spec.md) | `docs/spec.md`: the contract/runtime interface, implemented and planned | 0 |
| [003](003-division-checks.md) | division/modulo by zero and `min / -1` revert like solc | 1 |
| [004](004-keccak-and-mappings.md) | `__sys_keccak256` on SP1's keccak precompile; solc mapping slots; no storage packing | 1 |
| [005](005-transaction-context.md) | `msg.sender`/`msg.value`/`address(this)`/block info from a host-supplied context; non-payable check; shared `prova-common` types | 1 |
| [006](006-events.md) | Ethereum-style events through `__sys_log`; logs in the public values | 1 |
| [007](007-dynamic-abi.md) | `string`/`bytes` in the Ethereum ABI, `Error(string)`, solc-style calldata checks | 1 |
| [008](008-erc20.md) | `contracts/erc20.sol` and its test; `error "msg"` expectation | 1 |
