use clap::{ArgMatches, Command};
use magic_crypt::{new_magic_crypt, MagicCryptTrait};
use rpassword::read_password;
use std::io::{Read, Write};
use zeroize::Zeroizing;

use crate::utils;
// use prettytable::{cell, row, Table};
// use web3::types::U256;

pub const COMMAND: &str = "enc";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(COMMAND).about("Encrypt data with a password")
}

pub async fn call_sub_commands(_: &ArgMatches) {
    let str_password = {
        let password = utils::password::prompt_password("Type a password: ").unwrap_or_else(|e| {
            tracing::error!(error = %e);
            panic!()
        });

        let bytes = password.reveal().to_vec();
        Zeroizing::new(String::from_utf8(bytes).unwrap())
    };

    let str_private_key_password = {
        let private_key = utils::password::prompt_password("Paste the wallet private key: ")
            .unwrap_or_else(|e| {
                tracing::error!(error = %e);
                panic!()
            });

        let bytes = private_key.reveal().to_vec();
        Zeroizing::new(String::from_utf8(bytes).unwrap())
    };

    let mc = new_magic_crypt!(str_password, 256);
    let base64 = mc.encrypt_str_to_base64(str_private_key_password);

    println!("Encrypted key as base64: {}", base64);
}
