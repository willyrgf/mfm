pub mod config;
pub mod debug;
pub mod telegram;

pub trait Notifier {
    fn send(&self, message: String) -> Result<(), anyhow::Error>;
}

pub fn notify<T: Notifier>(notifier: T, message: String) -> Result<(), anyhow::Error> {
    notifier.send(message)
}

pub fn notify_all<T: Notifier>(notifiers: &[T], message: String) -> Result<(), anyhow::Error> {
    notifiers
        .iter()
        .try_for_each(|notifier| -> Result<(), anyhow::Error> {
            notifier.send(message.clone())?;
            Ok(())
        })
}
