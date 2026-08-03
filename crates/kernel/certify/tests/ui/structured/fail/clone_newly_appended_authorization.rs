use mfm_store::structured::NewlyAppendedAuthorization;

fn clone_authorization(authorization: NewlyAppendedAuthorization) {
    let _duplicate = authorization.clone();
}

fn main() {}
