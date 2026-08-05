use mfm_runtime::history::RuntimeHistoryPort;
use mfm_store::structured::PublicPhysicalBindingVerifier;

struct FakeHistory;
struct FakeBinding;

fn requires_history_port<T: RuntimeHistoryPort>() {}
fn requires_binding_verifier<T: PublicPhysicalBindingVerifier>() {}

fn main() {
    requires_history_port::<FakeHistory>();
    requires_binding_verifier::<FakeBinding>();
}
