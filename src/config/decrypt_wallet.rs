use super::{wallet::Wallets, Config};
use magic_crypt::{new_magic_crypt, MagicCryptTrait};
use rpassword::read_password;
use std::io::Write;

pub fn decrypt_wallets_from_config(c: Config) -> Config {
    let mut config = c;
    let mut wallets = config.wallets.hashmap().clone();
    for (k, wallet) in config
        .wallets
        .hashmap()
        .iter()
        .filter(|(_, v)| v.encrypted.unwrap_or(false) == true)
    {
        let mut n_wallet = wallet.clone();
        let decrypted = ask_password_and_decrypt(wallet.private_key.clone());
        n_wallet.private_key = decrypted;
        wallets.insert(k.clone(), n_wallet);
    }
    config.wallets = Wallets { 0: wallets };
    config
}

pub fn ask_password_and_decrypt(private_key: String) -> String {
    println!("Type a password: ");
    std::io::stdout().flush().unwrap();
    let password = read_password().unwrap();

    let mc = new_magic_crypt!(password, 256);
    // mc.decrypt_base64_to_string(private_key).unwrap()
    match mc.decrypt_base64_to_string(private_key) {
        Ok(s) => s,
        _ => panic!("invalid password"),
    }
}
