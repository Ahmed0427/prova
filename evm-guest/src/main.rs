//! The benchmark's baseline: the same transaction as the native guest, run
//! as solc bytecode by revm. It reads an `EvmInput` and commits the same
//! `Public` as the native guest, so the host can compare the two.

#![no_main]
sp1_zkvm::entrypoint!(main);

use prova_common::{Account, EvmInput, Log, Public, State, Word};
use revm::context::result::{ExecutionResult, Output};
use revm::context::{Context, TxEnv};
use revm::database::{CacheDB, EmptyDB};
use revm::primitives::{Address, Bytes, TxKind, U256};
use revm::state::{AccountInfo, Bytecode};
use revm::{DatabaseCommit, ExecuteEvm, MainBuilder, MainContext};
use sp1_zkvm::io::{commit, read};
use std::collections::BTreeMap;

pub fn main() {
    println!("cycle-tracker-report-start: setup");
    let EvmInput { input, codes } = read();
    let code = |name: &str| {
        codes
            .iter()
            .find(|code| code.name == name)
            .unwrap_or_else(|| panic!("no EVM code for {name}"))
    };

    // The starting state, as revm accounts: runtime code and storage.
    let mut db = CacheDB::new(EmptyDB::default());
    for account in &input.state {
        let address = Address::from(account.address);
        let bytecode = Bytecode::new_raw(Bytes::from(code(&account.code).runtime.clone()));
        db.insert_account_info(address, AccountInfo::from_bytecode(bytecode));
        for (slot, value) in &account.storage {
            db.insert_account_storage(
                address,
                U256::from_be_bytes(*slot),
                U256::from_be_bytes(*value),
            )
            .unwrap();
        }
    }

    let context = &input.context;
    let address = Address::from(context.address);
    // The native runtime has no balances; here the sender can always pay the
    // value it sends.
    if !input.state.iter().any(|a| a.address == context.caller) {
        let info = AccountInfo {
            balance: U256::MAX >> 1,
            ..Default::default()
        };
        db.insert_account_info(Address::from(context.caller), info);
    }
    let exists = input.state.iter().any(|a| a.address == context.address);
    // A deploy runs the creation code at the address itself: the code is the
    // init code followed by the constructor arguments, as for CREATE, and
    // what it returns becomes the account's code.
    let calldata = match &input.deploy {
        Some(name) if !exists => {
            let mut init = code(name).init.clone();
            init.extend_from_slice(&input.calldata);
            let bytecode = Bytecode::new_raw(init.into());
            db.insert_account_info(address, AccountInfo::from_bytecode(bytecode));
            Bytes::new()
        }
        _ => Bytes::from(input.calldata.clone()),
    };

    let mut evm = Context::mainnet()
        .with_db(db)
        .modify_cfg_chained(|cfg| {
            cfg.disable_nonce_check = true;
            cfg.disable_balance_check = true;
            cfg.disable_base_fee = true;
            cfg.disable_block_gas_limit = true;
            cfg.disable_fee_charge = true;
            cfg.tx_gas_limit_cap = Some(u64::MAX);
        })
        .modify_block_chained(|block| {
            block.number = U256::from(context.block_number);
            block.timestamp = U256::from(context.timestamp);
            block.gas_limit = u64::MAX;
        })
        .build_mainnet();
    let tx = TxEnv::builder()
        .caller(Address::from(context.caller))
        .kind(TxKind::Call(address))
        .value(U256::from_be_bytes(context.value))
        .data(calldata)
        .gas_limit(1 << 48)
        .gas_price(0)
        .build_fill();
    println!("cycle-tracker-report-end: setup");

    // As on Ethereum (and in the native guest), deploying over code fails.
    let deploying_over_code = input.deploy.is_some() && exists;
    println!("cycle-tracker-report-start: execute");
    let outcome = (!deploying_over_code).then(|| evm.transact(tx).expect("revm failed"));
    println!("cycle-tracker-report-end: execute");

    println!("cycle-tracker-report-start: commit");
    let reverted_state = || (true, Vec::new(), input.state.clone());
    let (reverted, output, logs, state_after) = match outcome {
        None => {
            let (reverted, logs, state) = reverted_state();
            (reverted, Vec::new(), logs, state)
        }
        Some(outcome) => match outcome.result {
            ExecutionResult::Success { output, logs, .. } => {
                let output = match output {
                    Output::Call(bytes) | Output::Create(bytes, _) => bytes.to_vec(),
                };
                let db = &mut evm.ctx.journaled_state.database;
                db.commit(outcome.state);
                let mut state = state_after(&input.state, db);
                if let Some(name) = &input.deploy {
                    assert_eq!(
                        output,
                        code(name).runtime,
                        "the constructor returned other runtime code (immutables?)"
                    );
                    state.push(Account {
                        address: context.address,
                        code: name.clone(),
                        storage: storage_of(db, address),
                    });
                    state.sort_by_key(|account| account.address);
                }
                let logs = logs
                    .into_iter()
                    .map(|log| Log {
                        address: log.address.into_array(),
                        topics: log.data.topics().iter().map(|t| t.0).collect(),
                        data: log.data.data.to_vec(),
                    })
                    .collect();
                // A deploy returns the runtime code; the native guest returns nothing.
                let output = if input.deploy.is_some() {
                    Vec::new()
                } else {
                    output
                };
                (false, output, logs, state)
            }
            ExecutionResult::Revert { output, .. } => {
                let (reverted, logs, state) = reverted_state();
                (reverted, output.to_vec(), logs, state)
            }
            ExecutionResult::Halt { reason, .. } => {
                println!("halt: {reason:?}");
                let (reverted, logs, state) = reverted_state();
                (reverted, Vec::new(), logs, state)
            }
        },
    };

    commit(&Public {
        context: input.context.clone(),
        deploy: input.deploy.clone(),
        calldata: input.calldata.clone(),
        state_before: input.state.clone(),
        reverted,
        output,
        logs,
        state_after,
    });
    println!("cycle-tracker-report-end: commit");
}

/// The accounts of `before`, with their storage as `db` now has it.
fn state_after(before: &State, db: &CacheDB<EmptyDB>) -> State {
    before
        .iter()
        .map(|account| Account {
            address: account.address,
            code: account.code.clone(),
            storage: storage_of(db, Address::from(account.address)),
        })
        .collect()
}

/// An account's storage, sorted by slot.
fn storage_of(db: &CacheDB<EmptyDB>, address: Address) -> Vec<(Word, Word)> {
    let slots: BTreeMap<Word, Word> = db
        .cache
        .accounts
        .get(&address)
        .map(|account| {
            account
                .storage
                .iter()
                .map(|(slot, value)| (slot.to_be_bytes(), value.to_be_bytes()))
                .collect()
        })
        .unwrap_or_default();
    slots.into_iter().collect()
}
