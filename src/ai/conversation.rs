use chrono::{DateTime, Local};
use std::collections::VecDeque;

/// Role in the conversation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MessageRole {
    User,
    Assistant,
    System,
}

/// A single message in the conversation.
#[derive(Debug, Clone)]
pub struct Message {
    pub role: MessageRole,
    pub content: String,
    pub timestamp: DateTime<Local>,
}

impl Message {
    pub fn user(content: &str) -> Self {
        Self {
            role: MessageRole::User,
            content: content.to_string(),
            timestamp: Local::now(),
        }
    }

    pub fn assistant(content: &str) -> Self {
        Self {
            role: MessageRole::Assistant,
            content: content.to_string(),
            timestamp: Local::now(),
        }
    }

    pub fn system(content: &str) -> Self {
        Self {
            role: MessageRole::System,
            content: content.to_string(),
            timestamp: Local::now(),
        }
    }
}

/// Manages the conversation history with the AI.
/// Keeps a rolling window to avoid blowing up the context.
#[derive(Debug)]
pub struct Conversation {
    pub messages: VecDeque<Message>,
    max_history: usize,
}

impl Conversation {
    pub fn new(max_history: usize) -> Self {
        Self {
            messages: VecDeque::new(),
            max_history,
        }
    }

    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push_back(Message::user(content));
        self.trim();
    }

    #[allow(dead_code)]
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push_back(Message::assistant(content));
        self.trim();
    }

    pub fn add_system_message(&mut self, content: &str) {
        self.messages.push_back(Message::system(content));
        self.trim();
    }

    /// Append a chunk to the last assistant message (for streaming).
    pub fn append_to_last_assistant(&mut self, chunk: &str) {
        if let Some(last) = self.messages.back_mut() {
            if last.role == MessageRole::Assistant {
                last.content.push_str(chunk);
                return;
            }
        }
        // No existing assistant message to append to, create one
        self.messages.push_back(Message::assistant(chunk));
    }

    /// Get messages formatted for the Claude API.
    pub fn to_api_messages(&self) -> Vec<serde_json::Value> {
        self.messages
            .iter()
            .filter(|m| m.role != MessageRole::System)
            .map(|m| {
                serde_json::json!({
                    "role": match m.role {
                        MessageRole::User => "user",
                        MessageRole::Assistant => "assistant",
                        MessageRole::System => "user", // shouldn't happen due to filter
                    },
                    "content": m.content,
                })
            })
            .collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
    }

    fn trim(&mut self) {
        while self.messages.len() > self.max_history {
            self.messages.pop_front();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Message constructors ──────────────────────────────────────

    #[test]
    fn message_user() {
        let m = Message::user("hello");
        assert_eq!(m.role, MessageRole::User);
        assert_eq!(m.content, "hello");
    }

    #[test]
    fn message_assistant() {
        let m = Message::assistant("hi there");
        assert_eq!(m.role, MessageRole::Assistant);
        assert_eq!(m.content, "hi there");
    }

    #[test]
    fn message_system() {
        let m = Message::system("context");
        assert_eq!(m.role, MessageRole::System);
        assert_eq!(m.content, "context");
    }

    // ── Conversation basics ───────────────────────────────────────

    #[test]
    fn new_conversation_empty() {
        let c = Conversation::new(10);
        assert_eq!(c.messages.len(), 0);
    }

    #[test]
    fn add_user_message() {
        let mut c = Conversation::new(10);
        c.add_user_message("test");
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].role, MessageRole::User);
        assert_eq!(c.messages[0].content, "test");
    }

    #[test]
    fn add_assistant_message() {
        let mut c = Conversation::new(10);
        c.add_assistant_message("response");
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].role, MessageRole::Assistant);
    }

    #[test]
    fn add_system_message() {
        let mut c = Conversation::new(10);
        c.add_system_message("sys info");
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].role, MessageRole::System);
    }

    // ── Trimming ──────────────────────────────────────────────────

    #[test]
    fn trims_to_max_history() {
        let mut c = Conversation::new(3);
        c.add_user_message("1");
        c.add_assistant_message("2");
        c.add_user_message("3");
        c.add_assistant_message("4");
        assert_eq!(c.messages.len(), 3);
        // Oldest should have been removed
        assert_eq!(c.messages[0].content, "2");
    }

    #[test]
    fn trim_removes_from_front() {
        let mut c = Conversation::new(2);
        c.add_user_message("first");
        c.add_user_message("second");
        c.add_user_message("third");
        assert_eq!(c.messages.len(), 2);
        assert_eq!(c.messages[0].content, "second");
        assert_eq!(c.messages[1].content, "third");
    }

    // ── Streaming append ──────────────────────────────────────────

    #[test]
    fn append_to_last_assistant_extends() {
        let mut c = Conversation::new(10);
        c.add_assistant_message("Hello");
        c.append_to_last_assistant(" World");
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].content, "Hello World");
    }

    #[test]
    fn append_creates_new_if_last_not_assistant() {
        let mut c = Conversation::new(10);
        c.add_user_message("question");
        c.append_to_last_assistant("response chunk");
        assert_eq!(c.messages.len(), 2);
        assert_eq!(c.messages[1].role, MessageRole::Assistant);
        assert_eq!(c.messages[1].content, "response chunk");
    }

    #[test]
    fn append_creates_new_if_empty() {
        let mut c = Conversation::new(10);
        c.append_to_last_assistant("first chunk");
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].role, MessageRole::Assistant);
    }

    // ── to_api_messages ───────────────────────────────────────────

    #[test]
    fn api_messages_filters_system() {
        let mut c = Conversation::new(10);
        c.add_system_message("hidden");
        c.add_user_message("visible");
        c.add_assistant_message("also visible");
        let api = c.to_api_messages();
        assert_eq!(api.len(), 2);
        assert_eq!(api[0]["role"], "user");
        assert_eq!(api[1]["role"], "assistant");
    }

    #[test]
    fn api_messages_correct_format() {
        let mut c = Conversation::new(10);
        c.add_user_message("hello");
        let api = c.to_api_messages();
        assert_eq!(api[0]["role"], "user");
        assert_eq!(api[0]["content"], "hello");
    }

    // ── clear ─────────────────────────────────────────────────────

    #[test]
    fn clear_empties_conversation() {
        let mut c = Conversation::new(10);
        c.add_user_message("1");
        c.add_assistant_message("2");
        c.clear();
        assert_eq!(c.messages.len(), 0);
    }
}
