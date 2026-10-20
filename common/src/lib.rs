//! What the host and the guest exchange: the guest's input, and the public
//! values it commits. Both are bincode, written and read as one value each,
//! so these types are the format (see docs/spec.md).

#![no_std]
extern crate alloc;

use alloc::string::String;
use alloc::vec::Vec;
use serde::{Deserialize, Serialize};

pub type Word = [u8; 32];
pub type Address = [u8; 20];
pub type Storage = Vec<(Word, Word)>;

/// The transaction context. The host chooses it, and the guest only makes it
/// public: nothing proves it is a real transaction (no signature, no chain).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Context {
    /// `msg.sender`
    pub caller: Address,
    /// `msg.value`, big-endian
    pub value: Word,
    /// `address(this)`
    pub address: Address,
    /// `block.number`
    pub block_number: u64,
    /// `block.timestamp`
    pub timestamp: u64,
}

/// A contract account: which linked contract runs at `address`, and its
/// storage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub address: Address,
    /// The contract's name, as linked into the guest (`build/contracts.txt`).
    pub code: String,
    /// Big-endian `(slot, value)` pairs, sorted by slot.
    pub storage: Storage,
}

/// Every contract account, sorted by address.
pub type State = Vec<Account>;

/// The guest's only input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub context: Context,
    /// `Some(name)`: deploy that contract at `context.address`, running its
    /// constructor. `None`: call the contract at `context.address`.
    pub deploy: Option<String>,
    pub calldata: Vec<u8>,
    /// The state the transaction starts from.
    pub state: State,
}

/// An event, as on Ethereum.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Log {
    /// The contract that emitted it.
    pub address: Address,
    /// At most 4.
    pub topics: Vec<Word>,
    pub data: Vec<u8>,
}

/// The guest's only commit: the public values a proof attests to.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Public {
    pub context: Context,
    pub deploy: Option<String>,
    pub calldata: Vec<u8>,
    pub state_before: State,
    pub reverted: bool,
    /// Return data, or revert data.
    pub output: Vec<u8>,
    /// The events emitted, in order; none after a revert.
    pub logs: Vec<Log>,
    /// Equal to `state_before` after a revert.
    pub state_after: State,
}
