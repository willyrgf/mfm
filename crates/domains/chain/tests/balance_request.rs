use std::error::Error;

use mfm_chain::balance::{
    BalanceRequest, BalanceRequestError, BalanceSource, BalanceTextError, DecimalScale,
};
use mfm_chain::{BalanceTarget, LedgerIdentity};
use mfm_values::{Object, Unsigned256};

#[test]
fn request_preserves_source_order_and_rejects_duplicate_or_foreign_ledger_sources() {
    // Scalar payloads exercise shared envelope admission, not EVM target qualification.
    let native = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let ledger = LedgerIdentity::new(native.clone());
    let target = BalanceTarget::new(ledger, native.clone());
    let sources = vec![
        BalanceSource::new("second".into(), target.clone()).unwrap(),
        BalanceSource::new("first".into(), target.clone()).unwrap(),
    ];
    let request = BalanceRequest::new(sources, DecimalScale::new(30).unwrap()).unwrap();
    let restored = Object::from_value(&request)
        .unwrap()
        .decode::<BalanceRequest>()
        .unwrap();
    assert_eq!(restored, request);
    assert_eq!(restored.sources()[0].source_id(), "second");
    assert_eq!(restored.sources()[1].source_id(), "first");
    assert_eq!(restored.decimals().get(), 30);

    let mut duplicate = request.sources().to_vec();
    duplicate.push(duplicate[0].clone());
    assert!(matches!(
        BalanceRequest::new(duplicate, request.decimals()),
        Err(BalanceRequestError::Duplicate {
            first: 0,
            duplicate: 2
        })
    ));
    let foreign = BalanceTarget::new(
        LedgerIdentity::new(Object::from_value(&Unsigned256::from_u64(2)).unwrap()),
        native,
    );
    let mut sources = request.sources().to_vec();
    sources.push(BalanceSource::new("foreign".into(), foreign.clone()).unwrap());
    assert!(matches!(
        BalanceRequest::new(sources, request.decimals()),
        Err(BalanceRequestError::Ledger { index: 2 })
    ));

    let wire = serde_json::to_value(&request).unwrap();
    let mut duplicate = wire.clone();
    duplicate["sources"][1]["source_id"] = duplicate["sources"][0]["source_id"].clone();
    let mut foreign_wire = wire.clone();
    foreign_wire["sources"][1]["target"] = serde_json::to_value(foreign).unwrap();
    let mut empty = wire.clone();
    empty["sources"] = serde_json::json!([]);
    let mut scale = wire;
    scale["decimals"] = serde_json::json!(31);
    for invalid in [duplicate, foreign_wire, empty, scale] {
        assert!(serde_json::from_str::<BalanceRequest>(&invalid.to_string()).is_err());
    }
}

#[test]
fn admission_rejects_invalid_public_ids_and_scales_without_retaining_rejected_text() {
    let native = Object::from_value(&Unsigned256::from_u64(1)).unwrap();
    let target = BalanceTarget::new(LedgerIdentity::new(native.clone()), native);
    for id in ["", "bad\nidentifier", "password=deliberate-test-marker"] {
        let error = BalanceSource::new(id.into(), target.clone()).unwrap_err();
        let diagnostic = serde_json::to_string(&error).unwrap();
        if !id.is_empty() {
            assert!(!diagnostic.contains(id));
        }
        let wire = serde_json::json!({ "source_id": id, "target": target });
        assert!(serde_json::from_str::<BalanceSource>(&wire.to_string()).is_err());
    }
    let error = BalanceSource::new("x".repeat(257), target).unwrap_err();
    assert!(error.source().is_some());
    let BalanceTextError::Size(size) = error else {
        panic!("expected retained size cause");
    };
    assert_eq!((size.actual(), size.limit()), (257, 256));
    for scale in [0, 30] {
        assert_eq!(DecimalScale::new(scale).unwrap().get(), scale);
    }
    for scale in [31, 255] {
        assert!(DecimalScale::new(scale).is_err());
        assert!(serde_json::from_str::<DecimalScale>(&scale.to_string()).is_err());
    }
    assert!(matches!(
        BalanceRequest::new(vec![], DecimalScale::new(0).unwrap()),
        Err(BalanceRequestError::Empty)
    ));
}
