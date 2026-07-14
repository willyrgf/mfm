use super::*;

macro_rules! accepts {
    ($ty:ty, $value:expr) => {
        assert_eq!(<$ty>::new($value).expect($value).as_ref(), $value);
    };
}

macro_rules! rejects {
    ($ty:ty, $value:expr) => {
        assert!(<$ty>::new($value).is_err(), "{:?}", $value);
    };
}

#[test]
fn checked_string_primitives_cover_shared_grammars() {
    accepts!(NameToken, "mfm.kernel/value_1");
    accepts!(StableAuthorKey, "portfolio/main-wallet");
    accepts!(FieldPath, "result.total/value");
    accepts!(FieldSegment, "total/value");
    accepts!(ResourceNamespace, "mfm.evm_lane");
    accepts!(LocalPublicId, "ethereum-mainnet");
    accepts!(RuntimeEnvName, "MFM_SECRET_1");
    accepts!(RuntimeBindingId, "mfm.portfolio/runtime:v1");
    accepts!(RuntimeToken, "admission_waiter:0123456789abcdef");
    accepts!(VisibleAscii256, "text/html");
    accepts!(VisibleAscii512, "commit-key");
    accepts!(PrintableAscii512, "redacted message");
    accepts!(PrintableAscii1024, "manual resolution note");

    rejects!(NameToken, "_name");
    rejects!(StableAuthorKey, "mfm.reserved");
    rejects!(FieldPath, "result..total");
    rejects!(ResourceNamespace, "single");
    rejects!(LocalPublicId, "bad/slash");
    rejects!(RuntimeEnvName, "mfm_secret");
    rejects!(RuntimeBindingId, "bad space");
    rejects!(RuntimeToken, "bad\nline");
    rejects!(VisibleAscii256, "has space");
    rejects!(PrintableAscii512, "line\nbreak");
    rejects!(PrintableAscii1024, "line\nbreak");
}

#[test]
fn short_stable_id_fragment_uses_alphanumeric_suffix() {
    assert_eq!(
        short_stable_id_fragment("content:sha256-jcs-v1:0123-45zz", 6),
        "012345"
    );
}
