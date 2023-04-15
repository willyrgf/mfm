use futures::executor;
use teloxide::{
    requests::{Request, Requester},
    types::ChatId,
    Bot,
};

use super::Notifier;

#[derive(Debug)]
pub struct TelegramNotifier {
    bot: Bot,
    chat_id: ChatId,
}

impl TelegramNotifier {
    pub fn new(token: String, chat_id: i64) -> Self {
        Self {
            bot: Bot::new(token),
            chat_id: ChatId(chat_id),
        }
    }
}

impl Notifier for TelegramNotifier {
    fn send(&self, message: String) -> Result<(), anyhow::Error> {
        let message = executor::block_on(self.bot.send_message(self.chat_id, message).send())
            .map_err(|e| anyhow::anyhow!("telegram send message error: {:?}", e))?;

        tracing::info!("telegram message sent: {:?}", message);

        Ok(())
    }
}
