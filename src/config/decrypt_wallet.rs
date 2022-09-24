use crate::utils::password::{decrypt_private_key, get_or_prompt_password};

use super::{wallet::Wallets, Config};

// TODO: refactor this mutable config when we have wallets as a mod with config and build types
pub fn decrypt_wallets_from_config(c: Config) -> Config {
    let mut config = c;
    let mut wallets = config.wallets.hashmap().clone();
    for (k, wallet) in config
        .wallets
        .hashmap()
        .iter()
        .filter(|(_, v)| v.encrypted.unwrap_or(false))
    {
        let mut n_wallet = wallet.clone();
        let decrypted = {
            let password = get_or_prompt_password(wallet.env_password.clone()).unwrap();
            decrypt_private_key(password, wallet.private_key.clone()).unwrap()
        };
        n_wallet.private_key = decrypted;
        wallets.insert(k.clone(), n_wallet);
    }
    config.wallets = Wallets(wallets);
    config
}
