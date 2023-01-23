use crate::utils::{self, password::encrypt_private_key_to_base64};
use clap::{ArgMatches, Command};

pub const COMMAND: &str = "enc";

pub fn generate_cmd<'a>() -> Command {
    Command::new(COMMAND).about("Encrypt data with a password")
}

pub async fn call_sub_commands(_: &ArgMatches) {
    let password = utils::password::prompt_password("Type a password: ").unwrap_or_else(|e| {
        tracing::error!(error = %e);
        panic!()
    });

    let private_key = utils::password::prompt_password("Paste the wallet private key: ")
        .unwrap_or_else(|e| {
            tracing::error!(error = %e);
            panic!()
        });

    let base64 = encrypt_private_key_to_base64(password, private_key);

    println!("Encrypted key as base64: {}", base64);
}
