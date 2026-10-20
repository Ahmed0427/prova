//! Test scripts (`tests/*.test`): steps, each followed by indented
//! expectations about its result.
//!
//! ```text
//! contract contracts/incrementer.sol     # read by ./prova, ignored here
//! deploy 5
//!     storage 0x0 = 5
//! get()
//!     returns 5
//! nope()
//!     reverts
//! ```
//!
//! A step must succeed unless it is followed by `reverts`.

use crate::{default_context, hex, parse_step, parse_word, short, Step};
use prova_common::{Public, Word};

pub struct Case {
    pub line: usize,
    pub step: Step,
    pub reverts: bool,
    pub expect: Vec<Expect>,
}

pub enum Expect {
    /// The exact output bytes: `returns WORD...` or `output 0xHEX`.
    Output(Vec<u8>),
    /// `storage SLOT = VALUE` after the step; 0 also matches a missing slot.
    Storage(Word, Word),
    /// `syscall NAME COUNT`: how often a precompile ran, e.g. `KECCAK_PERMUTE`.
    Syscall(String, u64),
}

pub fn parse(text: &str) -> Result<Vec<Case>, String> {
    let mut cases: Vec<Case> = Vec::new();

    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let content = raw.split('#').next().unwrap().trim_end();
        if content.trim().is_empty() {
            continue;
        }
        let at = |err: String| format!("line {line}: {err}");

        if !content.starts_with(char::is_whitespace) {
            if content.starts_with("contract ") {
                continue;
            }
            cases.push(Case {
                line,
                step: parse_step(content, &default_context()).map_err(at)?,
                reverts: false,
                expect: Vec::new(),
            });
            continue;
        }

        let case = cases
            .last_mut()
            .ok_or_else(|| at("an expectation needs a step above it".into()))?;
        let mut words = content.split_whitespace();
        match words.next().unwrap() {
            "reverts" => case.reverts = true,
            "returns" => {
                let mut output = Vec::new();
                for word in words {
                    output.extend_from_slice(&parse_word(word).map_err(at)?);
                }
                case.expect.push(Expect::Output(output));
            }
            "output" => {
                let bytes = words.next().unwrap_or("");
                let digits = bytes
                    .strip_prefix("0x")
                    .filter(|d| d.len() % 2 == 0)
                    .ok_or_else(|| {
                        at(format!(
                            "`{bytes}` is not 0x and an even number of hex digits"
                        ))
                    })?;
                let output = (0..digits.len())
                    .step_by(2)
                    .map(|i| u8::from_str_radix(&digits[i..i + 2], 16))
                    .collect::<Result<_, _>>()
                    .map_err(|err| at(err.to_string()))?;
                case.expect.push(Expect::Output(output));
            }
            "storage" => {
                let rest: Vec<&str> = words.collect();
                let [slot, "=", value] = rest[..] else {
                    return Err(at("write `storage SLOT = VALUE`".into()));
                };
                case.expect.push(Expect::Storage(
                    parse_word(slot).map_err(at)?,
                    parse_word(value).map_err(at)?,
                ));
            }
            "syscall" => {
                let rest: Vec<&str> = words.collect();
                let [name, count] = rest[..] else {
                    return Err(at("write `syscall NAME COUNT`".into()));
                };
                let count = count
                    .parse()
                    .map_err(|_| at(format!("`{count}` is not a count")))?;
                case.expect.push(Expect::Syscall(name.to_string(), count));
            }
            other => return Err(at(format!("unknown expectation `{other}`"))),
        }
    }
    Ok(cases)
}

/// Every way the result differs from what the case expects.
pub fn check(case: &Case, public: &Public, syscalls: &[(String, u64)]) -> Vec<String> {
    let mut failures = Vec::new();
    if public.reverted != case.reverts {
        failures.push(format!(
            "expected {}, but it {}",
            if case.reverts { "a revert" } else { "success" },
            if public.reverted {
                "reverted"
            } else {
                "succeeded"
            }
        ));
    }
    for expect in &case.expect {
        match expect {
            Expect::Output(output) if *output != public.output => failures.push(format!(
                "output is 0x{}, expected 0x{}",
                hex(&public.output),
                hex(output)
            )),
            Expect::Storage(slot, value) => {
                let actual = public
                    .storage_after
                    .iter()
                    .find(|(s, _)| s == slot)
                    .map_or([0; 32], |(_, v)| *v);
                if actual != *value {
                    failures.push(format!(
                        "storage {} is {}, expected {}",
                        short(slot),
                        short(&actual),
                        short(value)
                    ));
                }
            }
            Expect::Syscall(name, count) => {
                let actual = syscalls
                    .iter()
                    .find(|(n, _)| n == name)
                    .map_or(0, |(_, c)| *c);
                if actual != *count {
                    failures.push(format!("{name} ran {actual} times, expected {count}"));
                }
            }
            _ => {}
        }
    }
    failures
}
