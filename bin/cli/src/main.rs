use clap::Parser;

/// Standalone MFM command-line metadata surface.
#[derive(Parser)]
#[command(name = "mfm", version, about = "MFM standalone metadata")]
struct Cli {}

fn main() {
    let _ = Cli::parse();
    println!(
        "No trusted live composition is configured; admission and drive commands are unavailable."
    );
}
