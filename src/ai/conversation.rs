use chrono::{DateTime, Local};

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
    pub messages: Vec<Message>,
    max_history: usize,
}

impl Conversation {
    pub fn new(max_history: usize) -> Self {
        Self {
            messages: Vec::new(),
            max_history,
        }
    }

    pub fn add_user_message(&mut self, content: &str) {
        self.messages.push(Message::user(content));
        self.trim();
    }

    #[allow(dead_code)]
    pub fn add_assistant_message(&mut self, content: &str) {
        self.messages.push(Message::assistant(content));
        self.trim();
    }

    pub fn add_system_message(&mut self, content: &str) {
        self.messages.push(Message::system(content));
        self.trim();
    }

    /// Append a chunk to the last assistant message (for streaming).
    pub fn append_to_last_assistant(&mut self, chunk: &str) {
        if let Some(last) = self.messages.last_mut() {
            if last.role == MessageRole::Assistant {
                last.content.push_str(chunk);
                return;
            }
        }
        // No existing assistant message to append to, create one
        self.messages.push(Message::assistant(chunk));
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
            self.messages.remove(0);
        }
    }
}
