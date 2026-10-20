use prova_common::{Address, Context, Input, Public, Storage, Word};
use sp1_sdk::blocking::{CpuProver, ProveRequest, Prover, ProverClient};
use sp1_sdk::{include_elf, Elf, HashableKey, ProvingKey, SP1PublicValues, SP1Stdin};
use std::process::exit;
use std::time::Instant;
use tiny_keccak::{Hasher, Keccak};

mod abi;
mod script;

const ELF: Elf = include_elf!("prova-guest");

const USAGE: &str = "usage: prova-host [--prove] [OPTION VALUE]... STEP...
       prova-host --test FILE

each STEP is one transaction, run on the storage the previous one left:
  'deploy ARG...'          run the constructor
  'SIGNATURE ARG...'       call a function, e.g. 'inc(uint32) 5' or 'get()'
  '--from 0xa SIGNATURE ARG...'   with context options for this step only

each ARG is one 32-byte ABI word: 42, -1, true, false or 0xHEX (right-aligned).

context options (before the steps: the default for every step; inside one:
for that step), unauthenticated, only made public:
  --from ADDRESS        msg.sender       (default 0x1)
  --value N             msg.value        (default 0)
  --to ADDRESS          address(this)    (default 0xc0de)
  --block N             block.number     (default 1)
  --timestamp N         block.timestamp  (default 1)

with --prove, the last step is proven and verified instead of executed.
with --test, the steps and their expected results come from FILE.";

struct Step {
    label: String,
    context: Context,
    deploy: bool,
    calldata: Vec<u8>,
}

fn default_context() -> Context {
    Context {
        caller: address_from(1),
        value: [0; 32],
        address: address_from(0xc0de),
        block_number: 1,
        timestamp: 1,
    }
}

fn address_from(n: u64) -> Address {
    let mut address = [0; 20];
    address[12..].copy_from_slice(&n.to_be_bytes());
    address
}

/// Applies one context option; false if `name` isn't one.
fn set_option(context: &mut Context, name: &str, value: &str) -> Result<bool, String> {
    match name {
        "--from" => context.caller = parse_address(value)?,
        "--value" => context.value = parse_word(value)?,
        "--to" => context.address = parse_address(value)?,
        "--block" => context.block_number = parse_u64(value)?,
        "--timestamp" => context.timestamp = parse_u64(value)?,
        _ => return Ok(false),
    }
    Ok(true)
}

fn parse_u64(arg: &str) -> Result<u64, String> {
    arg.parse()
        .map_err(|_| format!("`{arg}` is not a 64-bit number"))
}

fn parse_address(arg: &str) -> Result<Address, String> {
    let word = parse_word(arg)?;
    if word[..12].iter().any(|b| *b != 0) {
        return Err(format!("`{arg}` is wider than an address"));
    }
    Ok(word[12..].try_into().unwrap())
}

fn print_public(public: &Public, defaults: &Context) {
    let context = &public.context;
    if context != defaults {
        println!(
            "  context         from {} value {} to {} block {} timestamp {}",
            short_address(&context.caller),
            short(&context.value),
            short_address(&context.address),
            context.block_number,
            context.timestamp
        );
    }
    println!("  storage before  {}", show_storage(&public.storage_before));
    println!("  reverted        {}", public.reverted);
    println!("  output          0x{}", hex(&public.output));
    for log in &public.logs {
        let topics: Vec<_> = log.topics.iter().map(short).collect();
        println!(
            "  log             {} [{}] 0x{}",
            short_address(&log.address),
            topics.join(", "),
            hex(&log.data)
        );
    }
    println!("  storage after   {}", show_storage(&public.storage_after));
}

fn short_address(address: &Address) -> String {
    let mut word = [0; 32];
    word[12..].copy_from_slice(address);
    short(&word)
}

fn fail(message: &str) -> ! {
    eprintln!("error: {message}\n\n{USAGE}");
    exit(2)
}

fn parse_step(text: &str, defaults: &Context) -> Result<Step, String> {
    let tokens = abi::tokenize(text)?;
    let mut parts = tokens.iter().map(String::as_str);
    let mut context = defaults.clone();
    let head = loop {
        let head = parts.next().ok_or("empty step")?;
        if !head.starts_with("--") {
            break head;
        }
        let value = parts.next().ok_or(format!("`{head}` needs a value"))?;
        if !set_option(&mut context, head, value)? {
            return Err(format!("unknown option `{head}`"));
        }
    };
    let deploy = head == "deploy";

    let mut calldata = if deploy {
        Vec::new()
    } else {
        if !head.contains('(') || !head.ends_with(')') {
            return Err(format!("`{head}` is not a function signature like `get()`"));
        }
        selector(head).to_vec()
    };
    calldata.extend_from_slice(&abi::encode(&abi::parse_values(parts)?));

    Ok(Step {
        label: text.trim().to_string(),
        context,
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
    keccak256(signature.as_bytes())[..4].try_into().unwrap()
}

fn keccak256(data: &[u8]) -> Word {
    let mut hash = [0; 32];
    let mut keccak = Keccak::v256();
    keccak.update(data);
    keccak.finalize(&mut hash);
    hash
}

fn stdin(step: &Step, storage: &Storage) -> SP1Stdin {
    let mut stdin = SP1Stdin::new();
    stdin.write(&Input {
        context: step.context.clone(),
        deploy: step.deploy,
        calldata: step.calldata.clone(),
        storage: storage.clone(),
    });
    stdin
}

fn read_public(values: &SP1PublicValues) -> Public {
    values.clone().read()
}

fn write_public(public: &Public) -> SP1PublicValues {
    let mut values = SP1PublicValues::new();
    values.write(public);
    values
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

/// One step run in SP1's executor.
struct Run {
    public: Public,
    /// What the guest printed: one line per `__sys_*` call.
    trace: String,
    cycles: u64,
    /// The syscalls other than I/O and halt, by name: the precompiles used.
    syscalls: Vec<(String, u64)>,
}

fn run(prover: &CpuProver, step: &Step, storage: &Storage) -> Run {
    let (guest_stdout, log) = tokio::sync::watch::channel(String::new());
    let (values, report) = prover
        .execute(ELF, stdin(step, storage))
        .stdout(guest_stdout)
        .run()
        .unwrap_or_else(|err| fail(&format!("the guest failed: {err}")));
    let trace = log.borrow().clone();
    let syscalls = report
        .syscall_counts
        .iter()
        .map(|(code, count)| (format!("{code:?}"), *count))
        .filter(|(name, count)| *count > 0 && !IO_SYSCALLS.contains(&name.as_str()))
        .collect();
    Run {
        public: read_public(&values),
        trace,
        cycles: report.total_instruction_count(),
        syscalls,
    }
}

const IO_SYSCALLS: [&str; 5] = ["WRITE", "HALT", "HINT_LEN", "HINT_READ", "COMMIT"];

fn show_syscalls(syscalls: &[(String, u64)]) -> String {
    let counts: Vec<_> = syscalls
        .iter()
        .map(|(name, count)| format!("{name} x{count}"))
        .collect();
    counts.join(", ")
}

fn execute(prover: &CpuProver, step: &Step, storage: &Storage) -> Public {
    println!("\n== {}", step.label);
    println!("  calldata        0x{}", hex(&step.calldata));

    let run = run(prover, step, storage);
    print!("{}", run.trace);
    println!("  cycles          {}", run.cycles);
    if !run.syscalls.is_empty() {
        println!("  precompiles     {}", show_syscalls(&run.syscalls));
    }
    print_public(&run.public, &default_context());
    run.public
}

/// Runs a test script; the steps share storage, as with `run`.
fn test(prover: &CpuProver, path: &str) -> bool {
    let text = std::fs::read_to_string(path).unwrap_or_else(|err| fail(&format!("{path}: {err}")));
    let cases = script::parse(&text).unwrap_or_else(|err| fail(&format!("{path}: {err}")));

    let mut storage = Storage::new();
    let mut passed = 0;
    for case in &cases {
        let run = run(prover, &case.step, &storage);
        let failures = script::check(case, &run.public, &run.syscalls);
        if failures.is_empty() {
            passed += 1;
            println!(
                "  ok    {:<50} {:>9} cycles",
                abbreviate(&case.step.label, 50),
                run.cycles
            );
        } else {
            println!("  FAIL  {} ({path}:{})", case.step.label, case.line);
            for failure in &failures {
                println!("        {failure}");
            }
            println!("  calldata        0x{}", hex(&case.step.calldata));
            print!("{}", run.trace);
            println!("  precompiles     {}", show_syscalls(&run.syscalls));
            print_public(&run.public, &default_context());
        }
        storage = run.public.storage_after;
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

    let public = read_public(&proof.public_values);
    print_public(&public, &default_context());

    let verified = prover.verify(&proof, pk.verifying_key(), None);
    println!(
        "\n  verify(proof)                      {}",
        verdict(verified.is_ok())
    );

    let forged = Public {
        reverted: !public.reverted,
        ..public
    };
    proof.public_values = write_public(&forged);
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
    let mut defaults = default_context();
    let mut steps = Vec::new();
    let mut args = args.iter().filter(|arg| *arg != "--prove");
    while let Some(arg) = args.next() {
        // A step can start with options too, but it is one quoted argument.
        if arg.starts_with("--") && !arg.contains(char::is_whitespace) {
            let value = args
                .next()
                .unwrap_or_else(|| fail(&format!("`{arg}` needs a value")));
            match set_option(&mut defaults, arg, value) {
                Ok(true) => {}
                Ok(false) => fail(&format!("unknown option `{arg}`")),
                Err(err) => fail(&err),
            }
        } else {
            steps.push(parse_step(arg, &defaults).unwrap_or_else(|err| fail(&err)));
        }
    }
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
