use std::error::Error;

use mfm_chain::balance::{
    BalanceArithmetic, BalanceCollectionFailure, BalanceRequest, BalanceSource, DecimalScale,
};
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_program::{Classification, ClassifyError};
use mfm_values::{Object, Unsigned256};

#[test]
fn scaling_keeps_canonical_zero_dust_exact_remainders_and_full_width_capacity() {
    let native = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let source = BalanceSource::new(
        "balance".into(),
        BalanceTarget::new(LedgerIdentity::new(native.clone()), native),
    )
    .unwrap();
    for (raw, source_scale, target_scale, expected) in [
        ("0", 0, 30, Some("0")),
        ("0", 30, 0, Some("0")),
        ("1", 0, 2, Some("100")),
        ("99", 2, 0, Some("0")),
        ("100", 2, 0, Some("1")),
        ("101", 2, 0, None),
        ("1000", 2, 0, Some("10")),
    ] {
        let request = BalanceRequest::new(
            vec![source.clone()],
            DecimalScale::new(target_scale).unwrap(),
        )
        .unwrap();
        let raw = Unsigned256::new(raw).unwrap();
        let result = request.scale_units(&raw, DecimalScale::new(source_scale).unwrap());
        if let Some(expected) = expected {
            assert_eq!(result.unwrap(), expected);
        } else {
            let failure = result.unwrap_err();
            assert!(matches!(
                failure,
                BalanceCollectionFailure::InexactScale { .. }
            ));
            assert_eq!(failure.classify(), Classification::Permanent);
            assert_eq!(
                Object::from_value(&failure)
                    .unwrap()
                    .decode::<BalanceCollectionFailure>()
                    .unwrap(),
                failure
            );
            let mut wire = serde_json::to_value(&failure).unwrap();
            wire["inexact_scale"]["raw_units"] = serde_json::json!("100");
            assert!(serde_json::from_value::<BalanceCollectionFailure>(wire).is_err());
        }
    }
    let max = Unsigned256::new(
        "115792089237316195423570985008687907853269984665640564039457584007913129639935",
    )
    .unwrap();
    let request = BalanceRequest::new(vec![source.clone()], DecimalScale::new(2).unwrap()).unwrap();
    assert_eq!(
        request
            .scale_units(&max, DecimalScale::new(0).unwrap())
            .unwrap(),
        format!("{max}00")
    );
    let request = BalanceRequest::new(vec![source], DecimalScale::new(3).unwrap()).unwrap();
    let failure = request
        .scale_units(&max, DecimalScale::new(0).unwrap())
        .unwrap_err();
    assert!(failure.source().is_some());
    let mut impossible = serde_json::to_value(&failure).unwrap();
    impossible["decimal_capacity_exceeded"]["size"]["actual"] = serde_json::json!(109);
    assert!(serde_json::from_value::<BalanceCollectionFailure>(impossible).is_err());
    assert_eq!(
        Object::from_value(&failure)
            .unwrap()
            .decode::<BalanceCollectionFailure>()
            .unwrap(),
        failure
    );
    let BalanceCollectionFailure::DecimalCapacityExceeded {
        operation, size, ..
    } = failure
    else {
        panic!("expected capacity rejection")
    };
    assert_eq!(operation, BalanceArithmetic::Scale);
    assert_eq!((size.actual(), size.limit()), (81, 80));
}

#[test]
fn totals_handle_increasing_width_and_report_the_first_overflowing_partial_sum() {
    let native = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let source = BalanceSource::new(
        "balance".into(),
        BalanceTarget::new(LedgerIdentity::new(native.clone()), native),
    )
    .unwrap();
    let scale = DecimalScale::new(0).unwrap();
    let request = BalanceRequest::new(vec![source.clone()], scale).unwrap();
    let small = Unsigned256::from_u64(9);
    let wide = Unsigned256::from_u64(100);
    assert_eq!(
        request
            .total_scaled([(&small, scale), (&wide, scale)])
            .unwrap(),
        "109"
    );
    assert_eq!(
        request
            .total_scaled([(&wide, scale), (&small, scale)])
            .unwrap(),
        "109"
    );
    assert_eq!(request.total_scaled([]).unwrap(), "0");

    let request = BalanceRequest::new(vec![source], DecimalScale::new(30).unwrap()).unwrap();
    let eighty_nines = Unsigned256::new("9".repeat(50)).unwrap();
    let one = Unsigned256::from_u64(1);
    let failure = request
        .total_scaled([(&eighty_nines, scale), (&one, scale)])
        .unwrap_err();
    assert_eq!(failure.classify(), Classification::Permanent);
    let restored = Object::from_value(&failure)
        .unwrap()
        .decode::<BalanceCollectionFailure>()
        .unwrap();
    assert_eq!(restored, failure);
    let BalanceCollectionFailure::DecimalCapacityExceeded {
        operation, size, ..
    } = &failure
    else {
        panic!("expected sum capacity rejection")
    };
    assert_eq!(*operation, BalanceArithmetic::Sum);
    assert_eq!((size.actual(), size.limit()), (81, 80));
    for (actual, limit) in [(80, 80), (81, 79), (82, 80)] {
        let mut wire = serde_json::to_value(&failure).unwrap();
        wire["decimal_capacity_exceeded"]["size"] =
            serde_json::json!({"actual":actual,"limit":limit});
        assert!(serde_json::from_value::<BalanceCollectionFailure>(wire).is_err());
    }
}
