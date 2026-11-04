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
    pub caller: Address,
    /// Big-endian.
    pub value: Word,
    pub address: Address,
    pub block_number: u64,
    pub timestamp: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub address: Address,
    pub code: String,
    /// Big-endian `(slot, value)` pairs, sorted by slot.
    pub storage: Storage,
}

/// Every contract account, sorted by address.
pub type State = Vec<Account>;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub context: Context,
    /// `Some(name)`: deploy that contract at `context.address`, running its
    /// constructor. `None`: call the contract at `context.address`.
    pub deploy: Option<String>,
    pub calldata: Vec<u8>,
    pub state: State,
    /// Print a line per `__sys_*` call. Off for benchmarks; not committed.
    pub trace: bool,
}

/// The benchmark's revm guest input: the same transaction plus solc bytecode.

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvmInput {
    pub input: Input,
    pub codes: Vec<EvmCode>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EvmCode {
    pub name: String,
    /// Creation code; the constructor arguments are appended to it.
    pub init: Vec<u8>,
    pub runtime: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Log {
    pub address: Address,
    /// At most 4.
    pub topics: Vec<Word>,
    pub data: Vec<u8>,
}

/// What the guest commits: the public values a proof attests to.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
