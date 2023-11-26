use mfm::{
    telemetry::{get_subscriber, init_subscriber},
    ExitCode, APP_NAME, DEFAULT_LOG_LEVEL,
};

fn main() {
    // This idiom is the prescribed way to get a clean shutdown of Rust (that will report
    // no leaks in Valgrind or sanitizers).  Calling `unsafe { libc::exit() }` does no
    // cleanup, and std::process::exit() does more--but does not run destructors.  So the
    // best thing to do is to is bubble up the exit code through the whole stack, and
    // only exit when everything potentially destructible has cleaned itself up.
    //
    // https://doc.rust-lang.org/std/process/fn.exit.html

    let exit_code = main_exitable();
    std::process::exit(exit_code as i32);
}

#[tracing::instrument(name = "main exitable")]
fn main_exitable() -> ExitCode {
    let subscriber = get_subscriber(APP_NAME.into(), DEFAULT_LOG_LEVEL.into(), std::io::stdout);
    init_subscriber(subscriber);

    unimplemented!()
}
