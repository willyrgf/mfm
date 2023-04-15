use serde::{Deserialize, Serialize};

use super::{telegram::TelegramNotifier, Notifier};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct Notifications(Vec<NotificationKind>);

impl Notifications {
    pub fn all_notifiers(&self) -> Vec<impl Notifier> {
        self.clone().0.iter().map(|n| n.notifier()).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", tag = "type")]
pub enum NotificationKind {
    Telegram { token: String, chat_id: i64 },
}

impl NotificationKind {
    pub fn notifier(&self) -> impl Notifier {
        match self {
            NotificationKind::Telegram { token, chat_id } => {
                TelegramNotifier::new(token.to_string(), *chat_id)
            }
        }
    }
}
