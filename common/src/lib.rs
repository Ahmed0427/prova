//! What the host and the guest exchange: the guest's input, and the public
//! values it commits. Both are bincode, written and read as one value each,
//! so these types are the format (see docs/spec.md).

#![no_std]
extern crate alloc;

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

/// The guest's only input.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Input {
    pub context: Context,
    /// `solang_deploy` (true) or `solang_call`.
    pub deploy: bool,
    pub calldata: Vec<u8>,
    /// The starting storage, big-endian `(slot, value)` pairs.
    pub storage: Storage,
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
    pub deploy: bool,
    pub calldata: Vec<u8>,
    pub storage_before: Storage,
    pub reverted: bool,
    /// Return data, or revert data.
    pub output: Vec<u8>,
    /// The events emitted, in order; none after a revert.
    pub logs: Vec<Log>,
    /// Sorted by slot; equal to `storage_before` after a revert.
    pub storage_after: Storage,
}
