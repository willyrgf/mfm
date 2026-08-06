use mfm_store::structured::TraceRunEvidence;

fn inspect(evidence: &TraceRunEvidence) {
    let _ = evidence.records()[0].request();
}

fn main() {}
