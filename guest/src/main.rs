#![no_main]
#![allow(static_mut_refs)]
sp1_zkvm::entrypoint!(main);

use sp1_zkvm::io::{commit, read};
use std::collections::BTreeMap;
use tiny_keccak::{Hasher, Keccak};

type Word = [u8; 32];

static mut STORAGE: BTreeMap<Word, Word> = BTreeMap::new();
static mut BEFORE: Vec<(Word, Word)> = Vec::new();

extern "C" {
    fn solang_deploy(input: *const u8, len: u32);
    fn solang_call(input: *const u8, len: u32);
}

pub fn main() {
    let deploy: bool = read();
    let calldata: Vec<u8> = read();
    let storage: Vec<(Word, Word)> = read();

    commit(&deploy);
    commit(&calldata);
    commit(&storage);

    unsafe {
        STORAGE = storage.iter().copied().collect();
        BEFORE = storage;

        let entry = if deploy { solang_deploy } else { solang_call };
        entry(calldata.as_ptr(), calldata.len() as u32);
    }

    unreachable!("contracts end with __sys_return or __sys_revert");
}

// Solang passes 256-bit values as four u64 limbs, least significant first.
fn to_word(limbs: [u64; 4]) -> Word {
    let mut word = [0; 32];
    for (i, limb) in limbs.iter().enumerate() {
        word[24 - 8 * i..32 - 8 * i].copy_from_slice(&limb.to_be_bytes());
    }
    word
}

fn to_limbs(word: &Word) -> [u64; 4] {
    core::array::from_fn(|i| u64::from_be_bytes(word[24 - 8 * i..32 - 8 * i].try_into().unwrap()))
}

#[no_mangle]
extern "C" fn __sys_sload(k0: u64, k1: u64, k2: u64, k3: u64, out: *mut [u64; 4]) {
    let key = to_word([k0, k1, k2, k3]);
    let value = unsafe { STORAGE.get(&key).copied().unwrap_or_default() };
    println!("  sload  {} -> {}", hex(&key), hex(&value));
    unsafe { *out = to_limbs(&value) };
}

#[no_mangle]
#[allow(clippy::too_many_arguments)]
extern "C" fn __sys_sstore(k0: u64, k1: u64, k2: u64, k3: u64, v0: u64, v1: u64, v2: u64, v3: u64) {
    let key = to_word([k0, k1, k2, k3]);
    let value = to_word([v0, v1, v2, v3]);
    println!("  sstore {} <- {}", hex(&key), hex(&value));
    unsafe { STORAGE.insert(key, value) };
}

#[no_mangle]
extern "C" fn __sys_keccak256(data: *const u8, len: u64, out: *mut [u8; 32]) {
    let data = bytes(data, len);
    let mut keccak = Keccak::v256();
    keccak.update(data);
    unsafe { keccak.finalize(&mut *out) };
    println!("  keccak 0x{} -> 0x{}", bytes_hex(data), bytes_hex(unsafe { &*out }));
}

#[no_mangle]
extern "C" fn __sys_return(data: *const u8, len: u64) -> ! {
    let storage = unsafe { STORAGE.iter().map(|(k, v)| (*k, *v)).collect() };
    finish(false, data, len, storage)
}

// A revert undoes every storage write.
#[no_mangle]
extern "C" fn __sys_revert(data: *const u8, len: u64) -> ! {
    finish(true, data, len, unsafe { BEFORE.clone() })
}

/// The contract's bytes; `data` may be null when `len` is 0.
fn bytes<'a>(data: *const u8, len: u64) -> &'a [u8] {
    if len == 0 {
        &[]
    } else {
        unsafe { core::slice::from_raw_parts(data, len as usize) }
    }
}

fn finish(reverted: bool, data: *const u8, len: u64, storage: Vec<(Word, Word)>) -> ! {
    let data = bytes(data, len).to_vec();

    println!(
        "  {} 0x{}",
        if reverted { "revert" } else { "return" },
        bytes_hex(&data)
    );
    commit(&reverted);
    commit(&data);
    commit(&storage);

    sp1_zkvm::syscalls::syscall_halt(0)
}

fn hex(word: &Word) -> String {
    let digits = bytes_hex(word);
    let digits = digits.trim_start_matches('0');
    format!("0x{}", if digits.is_empty() { "0" } else { digits })
}

fn bytes_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
