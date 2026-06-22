//! File-backed `ConversationStore`: one JSON file per conversation under a
//! directory. Survives restarts (unlike the in-memory store).

use crate::error::{AgentError, Result};
use crate::model::conversation::Conversation;
use crate::model::user::User;
use crate::traits::ConversationStore;
use async_trait::async_trait;
use std::path::PathBuf;

pub struct FileConversationStore {
    dir: PathBuf,
}

impl FileConversationStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        let _ = std::fs::create_dir_all(&dir);
        Self { dir }
    }

    fn path(&self, conversation_id: &str) -> Result<PathBuf> {
        // Guard against path traversal via a crafted conversation id.
        if conversation_id.contains(['/', '\\']) || conversation_id.contains("..") {
            return Err(AgentError::Validation(format!(
                "invalid conversation id: {conversation_id}"
            )));
        }
        Ok(self.dir.join(format!("{conversation_id}.json")))
    }
}

#[async_trait]
impl ConversationStore for FileConversationStore {
    async fn get_conversation(
        &self,
        conversation_id: &str,
        user: &User,
    ) -> Result<Option<Conversation>> {
        let path = self.path(conversation_id)?;
        match tokio::fs::read(&path).await {
            Ok(bytes) => {
                let conv: Conversation = serde_json::from_slice(&bytes)?;
                Ok((conv.user.id == user.id).then_some(conv))
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(AgentError::other(format!("read conversation: {e}"))),
        }
    }

    async fn update_conversation(&self, conversation: &Conversation) -> Result<()> {
        let path = self.path(&conversation.id)?;
        let bytes = serde_json::to_vec_pretty(conversation)?;
        tokio::fs::write(&path, bytes)
            .await
            .map_err(|e| AgentError::other(format!("write conversation: {e}")))
    }

    async fn delete_conversation(&self, conversation_id: &str, user: &User) -> Result<bool> {
        if self
            .get_conversation(conversation_id, user)
            .await?
            .is_none()
        {
            return Ok(false);
        }
        let path = self.path(conversation_id)?;
        tokio::fs::remove_file(&path)
            .await
            .map_err(|e| AgentError::other(format!("delete conversation: {e}")))?;
        Ok(true)
    }

    async fn list_conversations(
        &self,
        user: &User,
        limit: usize,
        offset: usize,
    ) -> Result<Vec<Conversation>> {
        let mut entries = tokio::fs::read_dir(&self.dir)
            .await
            .map_err(|e| AgentError::other(format!("list dir: {e}")))?;
        let mut convs = Vec::new();
        while let Some(entry) = entries
            .next_entry()
            .await
            .map_err(|e| AgentError::other(format!("dir entry: {e}")))?
        {
            if entry.path().extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if let Ok(bytes) = tokio::fs::read(entry.path()).await {
                if let Ok(conv) = serde_json::from_slice::<Conversation>(&bytes) {
                    if conv.user.id == user.id {
                        convs.push(conv);
                    }
                }
            }
        }
        convs.sort_by_key(|c| std::cmp::Reverse(c.updated_at));
        Ok(convs.into_iter().skip(offset).take(limit).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::conversation::Message;

    #[tokio::test]
    async fn roundtrip_and_user_scoping() {
        let dir = std::env::temp_dir().join(format!("gauss_store_{}", std::process::id()));
        let store = FileConversationStore::new(&dir);
        let alice = User::new("alice");
        let bob = User::new("bob");

        let mut conv = Conversation::new("c1", alice.clone());
        conv.add_message(Message::user("hello"));
        store.update_conversation(&conv).await.unwrap();

        // Alice can read it back; Bob cannot (user scoping).
        let got = store.get_conversation("c1", &alice).await.unwrap();
        assert_eq!(got.unwrap().messages.len(), 1);
        assert!(store.get_conversation("c1", &bob).await.unwrap().is_none());

        // Listing is scoped to the owner.
        assert_eq!(
            store.list_conversations(&alice, 10, 0).await.unwrap().len(),
            1
        );
        assert_eq!(
            store.list_conversations(&bob, 10, 0).await.unwrap().len(),
            0
        );

        // Path traversal is rejected.
        assert!(store.get_conversation("../escape", &alice).await.is_err());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
