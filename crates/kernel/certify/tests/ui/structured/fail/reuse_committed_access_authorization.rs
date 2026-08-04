use mfm_facts::FactSelectionRequest;
use mfm_certify::structured::CertifiedAccessAuthorization;

fn reuse(
    authorization: CertifiedAccessAuthorization,
    first_request: FactSelectionRequest,
    second_request: FactSelectionRequest,
) {
    let _first = authorization.invoke_prior_run_fact_scan(first_request);
    let _second = authorization.invoke_prior_run_fact_scan(second_request);
}

fn main() {}
