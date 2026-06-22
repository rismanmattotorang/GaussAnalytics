//! In-memory `ConversationStore`. Mirrors GaussAnalytics's `MemoryConversationStore`.

use crate::model::conversation::Conversation;
use crate::model::user::User;
use crate::traits::ConversationStore;
use crate::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::RwLock;

#[derive(Default)]
pub struct InMemoryConversationStore {
    conversations: RwLock<HashMap<String, Conversation>>,
}

impl InMemoryConversationStore {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ConversationStore for InMemoryConversationStore {
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user: &User,
    ) -> Result<Option<Conversation>> {
        let map = self.conversations.read().unwrap();
        Ok(map
            .get(conversation_id)
            .filter(|c| c.user.id == user.id)
            .cloned())
    }

    async fn update_conversation(&self, conversation: &Conversation) -> Result<()> {
        let mut map = self.conversations.write().unwrap();
        map.insert(conversation.id.clone(), conversation.clone());
        Ok(())
    }

    async fn delete_conversation(&self, conversation_id: &str, user: &User) -> Result<bool> {
        let mut map = self.conversations.write().unwrap();
        match map.get(conversation_id) {
            Some(c) if c.user.id == user.id => {
                map.remove(conversation_id);
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    async fn list_conversations(
        &self,
        user: &User,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Conversation>> {
        let map = self.conversations.read().unwrap();
        let mut owned: Vec<Conversation> = map
            .values()
            .filter(|c| c.user.id == user.id)
            .cloned()
            .collect();
        owned.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        Ok(owned.into_iter().skip(offset).take(limit).collect())
    }
}
