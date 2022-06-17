extern crate magic_crypt;
extern crate rpassword; // use crate::{cmd, shared};

use clap::{ArgMatches, Command};
use magic_crypt::{new_magic_crypt, MagicCryptTrait};
use rpassword::read_password;
use std::io::Write;
// use prettytable::{cell, row, Table};
// use web3::types::U256;

pub const COMMAND: &str = "enc";

pub fn generate_cmd<'a>() -> Command<'a> {
    Command::new(COMMAND).about("Encrypt data with a password")
}

pub async fn call_sub_commands(_: &ArgMatches) {
    println!("Type a password: ");
    std::io::stdout().flush().unwrap();
    let password = read_password().unwrap();
    //println!("The password is: '{}'", password);

    println!("Paste the wallet private key: ");
    let private_key = read_password().unwrap();

    let mc = new_magic_crypt!(password, 256);
    let base64 = mc.encrypt_str_to_base64(private_key);

    print!("ecrypted key as base64: {}", base64);
}
