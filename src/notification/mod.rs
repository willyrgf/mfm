pub mod debug;

pub trait Notification {
    fn notify(&self, message: String) -> Result<(), anyhow::Error>;
}
