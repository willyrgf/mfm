use std::env;
use std::fs;
use std::path::PathBuf;

use mfm_kernel_test_support::typed_certified_slice_summary;

#[tokio::main]
async fn main() {
    if let Err(error) = run().await {
        eprintln!("ERROR: {error}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), String> {
    let mut summary_file = None::<PathBuf>;
    let mut args = env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--summary-file" => {
                let Some(path) = args.next() else {
                    return Err("--summary-file requires a value".to_owned());
                };
                summary_file = Some(PathBuf::from(path));
            }
            "-h" | "--help" => {
                println!(
                    "usage: typed_certified_slice_summary [--summary-file PATH]\n\nRuns the typed-certified-slice acceptance harness and writes the RFC summary document."
                );
                return Ok(());
            }
            other => return Err(format!("unknown argument: {other}")),
        }
    }

    let summary = typed_certified_slice_summary().await?;
    let document = summary.to_summary_document();
    let bytes = serde_json::to_vec_pretty(&document).map_err(|error| error.to_string())?;
    if let Some(path) = summary_file {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| error.to_string())?;
        }
        fs::write(path, bytes).map_err(|error| error.to_string())?;
    } else {
        println!(
            "{}",
            String::from_utf8(bytes).map_err(|error| error.to_string())?
        );
    }
    Ok(())
}
