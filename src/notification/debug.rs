use super::Notifier;

#[derive(Default)]
pub struct DebugNotifier {}

impl Notifier for DebugNotifier {
    fn send(&self, message: String) -> Result<(), anyhow::Error> {
        tracing::info!("{}", message);
        Ok(())
    }
}
