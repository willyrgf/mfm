#[path = "../support.rs"]
mod support;

use mfm_capabilities::{AccessFaultCode, ReadAdapterCompletion};
use support::{stable, Output};

fn main() {
    let _: ReadAdapterCompletion<Output, Output> = ReadAdapterCompletion::EntryUnknown(
        AccessFaultCode::new(stable("mfm.ui/entry-unknown")),
    );
}
