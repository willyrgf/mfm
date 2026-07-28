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
    accepts!(EntryPointId, "mfm.a/b@1");
    accepts!(EntryPointId, "mfm.portfolio/snapshot@18446744073709551615");
    accepts!(FieldSegment, "total/value-1");
    accepts!(FieldPath, "result.total/value");
    accepts!(LocalPublicId, "ethereum-mainnet");
    accepts!(RuntimeEnvName, "MFM_SECRET_1");

    rejects!(NameToken, "_name");
    rejects!(StableAuthorKey, "mfm.reserved");
    rejects!(EntryPointId, "mfm.portfolio-/snapshot@1");
    rejects!(EntryPointId, "mfm.portfolio/snapshot_@1");
    rejects!(EntryPointId, "mfm.portfolio/snapshot@01");
    rejects!(EntryPointId, "mfm.portfolio/snapshot@0");
    rejects!(EntryPointId, "mfm.portfolio/snapshot@18446744073709551616");
    rejects!(FieldSegment, "");
    rejects!(FieldSegment, "nested.field");
    rejects!(FieldSegment, "_private");
    rejects!(FieldPath, "result..total");
    rejects!(LocalPublicId, "bad/slash");
    rejects!(RuntimeEnvName, "mfm_secret");
}

#[test]
fn short_stable_id_fragment_uses_alphanumeric_suffix() {
    assert_eq!(
        short_stable_id_fragment("content:sha256-jcs-v1:0123-45zz", 6),
        "012345"
    );
}
