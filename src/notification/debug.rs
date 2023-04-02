use super::Notification;

#[derive(Default)]
pub struct DebugNotification {}

impl Notification for DebugNotification {
    fn notify(&self, message: String) -> Result<(), anyhow::Error> {
        tracing::info!("{}", message);
        Ok(())
    }
}
