use std::process::ExitCode;

use mfm_aave_origin_compile::{
    compile_from_stdin, read_stdin_string, serialize_compile_manifest, CompileConfig,
};

fn main() -> ExitCode {
    let mut args = std::env::args().skip(1);
    if let Some(arg) = args.next() {
        if arg == "--help" || arg == "-h" {
            println!("mfm-aave-v3-origin-compile reads pinned origin source json from stdin and emits an evm_contract_set_compile_manifest_v1 to stdout.");
            return ExitCode::SUCCESS;
        }
        eprintln!("unexpected argument: {arg}");
        return ExitCode::from(2);
    }

    let stdin = match read_stdin_string() {
        Ok(stdin) => stdin,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };
    let cfg = CompileConfig::from_env();
    let manifest = match compile_from_stdin(&stdin, &cfg) {
        Ok(manifest) => manifest,
        Err(err) => {
            eprintln!("{err}");
            return ExitCode::FAILURE;
        }
    };

    match serialize_compile_manifest(&manifest) {
        Ok(output) => {
            println!("{output}");
            ExitCode::SUCCESS
        }
        Err(err) => {
            eprintln!("{err}");
            ExitCode::FAILURE
        }
    }
}
