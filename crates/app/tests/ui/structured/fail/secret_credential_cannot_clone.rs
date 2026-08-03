fn main() {
    let credential = mfm_app::SecretCredential::new(vec![1]).unwrap();
    let _copy = credential.clone();
}
