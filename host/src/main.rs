use sp1_sdk::blocking::{CpuProver, ProveRequest, Prover, ProverClient};
use sp1_sdk::{include_elf, Elf, HashableKey, ProvingKey, SP1PublicValues, SP1Stdin};
use std::process::exit;
use std::time::Instant;
use tiny_keccak::{Hasher, Keccak};

mod script;

const ELF: Elf = include_elf!("prova-guest");

type Word = [u8; 32];
type Storage = Vec<(Word, Word)>;

const USAGE: &str = "usage: prova-host [--prove] STEP...
       prova-host --test FILE

each STEP is one transaction, run on the storage the previous one left:
  'deploy ARG...'          run the constructor
  'SIGNATURE ARG...'       call a function, e.g. 'inc(uint32) 5' or 'get()'

each ARG is one 32-byte ABI word: 42, -1, true, false or 0xHEX (right-aligned).
with --prove, the last step is proven and verified instead of executed.
with --test, the steps and their expected results come from FILE.";

struct Step {
    label: String,
    deploy: bool,
    calldata: Vec<u8>,
}

#[derive(Clone)]
struct Public {
    deploy: bool,
    calldata: Vec<u8>,
    storage_before: Storage,
    reverted: bool,
    output: Vec<u8>,
    storage_after: Storage,
}

impl Public {
    fn read(values: &SP1PublicValues) -> Self {
        let mut values = values.clone();
        Public {
            deploy: values.read(),
            calldata: values.read(),
            storage_before: values.read(),
            reverted: values.read(),
            output: values.read(),
            storage_after: values.read(),
        }
    }

    fn write(&self) -> SP1PublicValues {
        let mut values = SP1PublicValues::new();
        values.write(&self.deploy);
        values.write(&self.calldata);
        values.write(&self.storage_before);
        values.write(&self.reverted);
        values.write(&self.output);
        values.write(&self.storage_after);
        values
    }

    fn print(&self) {
        println!("  storage before  {}", show_storage(&self.storage_before));
        println!("  reverted        {}", self.reverted);
        println!("  output          0x{}", hex(&self.output));
        println!("  storage after   {}", show_storage(&self.storage_after));
    }
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}\n\n{USAGE}");
    exit(2)
}

fn parse_step(text: &str) -> Result<Step, String> {
    let mut parts = text.split_whitespace();
    let head = parts.next().ok_or("empty step")?;
    let deploy = head == "deploy";

    let mut calldata = if deploy {
        Vec::new()
    } else {
        if !head.contains('(') || !head.ends_with(')') {
            return Err(format!("`{head}` is not a function signature like `get()`"));
        }
        selector(head).to_vec()
    };
    for arg in parts {
        calldata.extend_from_slice(&parse_word(arg)?);
    }

    Ok(Step {
        label: text.trim().to_string(),
        deploy,
        calldata,
    })
}

fn parse_word(arg: &str) -> Result<Word, String> {
    let bad = || format!("`{arg}` is not a number, true, false or 0xHEX");
    let mut word = [0; 32];

    if let Some(digits) = arg.strip_prefix("0x") {
        if digits.is_empty() || digits.len() > 64 {
            return Err(bad());
        }
        let digits = format!("{digits:0>64}");
        for (i, byte) in word.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&digits[2 * i..2 * i + 2], 16).map_err(|_| bad())?;
        }
    } else if arg == "true" || arg == "false" {
        word[31] = (arg == "true") as u8;
    } else if arg.starts_with('-') {
        let value: i128 = arg.parse().map_err(|_| bad())?;
        word = [0xff; 32];
        word[16..].copy_from_slice(&value.to_be_bytes());
    } else {
        let value: u128 = arg.parse().map_err(|_| bad())?;
        word[16..].copy_from_slice(&value.to_be_bytes());
    }
    Ok(word)
}

fn selector(signature: &str) -> [u8; 4] {
    let mut hash = [0; 32];
    let mut keccak = Keccak::v256();
    keccak.update(signature.as_bytes());
    keccak.finalize(&mut hash);
    hash[..4].try_into().unwrap()
}

fn stdin(step: &Step, storage: &Storage) -> SP1Stdin {
    let mut stdin = SP1Stdin::new();
    stdin.write(&step.deploy);
    stdin.write(&step.calldata);
    stdin.write(storage);
    stdin
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn short(word: &Word) -> String {
    let digits = hex(word);
    let digits = digits.trim_start_matches('0');
    format!("0x{}", if digits.is_empty() { "0" } else { digits })
}

fn show_storage(storage: &Storage) -> String {
    let slots: Vec<_> = storage
        .iter()
        .map(|(slot, value)| format!("{} = {}", short(slot), short(value)))
        .collect();
    format!("{{{}}}", slots.join(", "))
}

/// Runs one step in SP1's executor: the public values, the guest's trace
/// and the cycle count.
fn run(prover: &CpuProver, step: &Step, storage: &Storage) -> (Public, String, u64) {
    let (guest_stdout, log) = tokio::sync::watch::channel(String::new());
    let (values, report) = prover
        .execute(ELF, stdin(step, storage))
        .stdout(guest_stdout)
        .run()
        .unwrap_or_else(|err| fail(&format!("the guest failed: {err}")));
    let trace = log.borrow().clone();
    (
        Public::read(&values),
        trace,
        report.total_instruction_count(),
    )
}

fn execute(prover: &CpuProver, step: &Step, storage: &Storage) -> Public {
    println!("\n== {}", step.label);
    println!("  calldata        0x{}", hex(&step.calldata));

    let (public, trace, cycles) = run(prover, step, storage);
    print!("{trace}");
    println!("  cycles          {cycles}");
    public.print();
    public
}

/// Runs a test script; the steps share storage, as with `run`.
fn test(prover: &CpuProver, path: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_else(|err| fail(&format!("{path}: {err}")));
    let cases = script::parse(&text).unwrap_or_else(|err| fail(&format!("{path}: {err}")));

    let mut storage = Storage::new();
    let mut passed = 0;
    for case in &cases {
        let (public, trace, cycles) = run(prover, &case.step, &storage);
        let failures = script::check(case, &public);
        if failures.is_empty() {
            passed += 1;
            println!(
                "  ok    {:<50} {cycles:>9} cycles",
                abbreviate(&case.step.label, 50)
            );
        } else {
            println!("  FAIL  {} ({path}:{})", case.step.label, case.line);
            for failure in &failures {
                println!("        {failure}");
            }
            println!("  calldata        0x{}", hex(&case.step.calldata));
            print!("{trace}");
            public.print();
        }
        storage = public.storage_after;
    }
    println!("{path}: {passed}/{} steps passed", cases.len());
    passed == cases.len()
}

fn abbreviate(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let head: String = text.chars().take(width - 3).collect();
    format!("{head}...")
}

fn prove(prover: &CpuProver, step: &Step, storage: &Storage) {
    println!("\n== proving {}", step.label);

    let start = Instant::now();
    let pk = prover.setup(ELF).expect("setup failed");
    println!("  setup           {:.1?}", start.elapsed());
    println!("  vkey            {}", pk.verifying_key().bytes32());

    let start = Instant::now();
    let mut proof = prover
        .prove(&pk, stdin(step, storage))
        .core()
        .run()
        .expect("proving failed");
    println!(
        "  proved          {:.1?}, {} bytes",
        start.elapsed(),
        proof.bytes().len()
    );

    let public = Public::read(&proof.public_values);
    public.print();

    let verified = prover.verify(&proof, pk.verifying_key(), None);
    println!(
        "\n  verify(proof)                      {}",
        verdict(verified.is_ok())
    );

    let forged = Public {
        reverted: !public.reverted,
        ..public
    };
    proof.public_values = forged.write();
    let forged_verified = prover.verify(&proof, pk.verifying_key(), None);
    println!(
        "  verify(proof, reverted flipped)    {}",
        verdict(forged_verified.is_ok())
    );

    if verified.is_err() || forged_verified.is_ok() {
        exit(1);
    }
}

fn verdict(ok: bool) -> &'static str {
    if ok {
        "accepted"
    } else {
        "rejected"
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return;
    }

    let prover = ProverClient::builder().cpu().build();

    if args.first().is_some_and(|arg| arg == "--test") {
        let [_, path] = &args[..] else {
            fail("--test takes one file");
        };
        exit(if test(&prover, path) { 0 } else { 1 });
    }

    let prove_last = args.iter().any(|arg| arg == "--prove");
    let steps: Vec<Step> = args
        .iter()
        .filter(|arg| *arg != "--prove")
        .map(|arg| parse_step(arg).unwrap_or_else(|err| fail(&err)))
        .collect();
    if steps.is_empty() {
        fail("no steps given");
    }

    let mut storage = Storage::new();

    for (i, step) in steps.iter().enumerate() {
        if prove_last && i == steps.len() - 1 {
            prove(&prover, step, &storage);
        } else {
            storage = execute(&prover, step, &storage).storage_after;
        }
    }
}
