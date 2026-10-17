use std::collections::HashSet;

use crate::net::packets::Clientbound;

#[derive(Debug, Clone)]
pub struct PlayerChatEvent {
    pub player: String,
    pub message: String,
    pub format: String,
    pub cancelled: bool,
    pub recipients: HashSet<String>,
}

impl PlayerChatEvent {
    pub fn new(player: String, message: String, all_players: &[String]) -> Self {
        Self {
            player,
            message,
            format: "<%1$s> %2$s".to_string(),
            cancelled: false,
            recipients: all_players.iter().cloned().collect(),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn set_cancelled(&mut self, cancel: bool) {
        self.cancelled = cancel;
    }

    pub fn get_format(&self) -> &str {
        &self.format
    }

    pub fn set_format(&mut self, format: String) {
        self.format = format;
    }

    pub fn get_recipients(&self) -> &HashSet<String> {
        &self.recipients
    }
}

#[derive(Debug, Clone)]
pub struct PlayerCommandPreprocessEvent {
    pub inner: PlayerChatEvent,
}

impl PlayerCommandPreprocessEvent {
    pub fn new(player: String, message: String, all_players: &[String]) -> Self {
        Self {
            inner: PlayerChatEvent::new(player, message, all_players),
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled
    }

    pub fn set_cancelled(&mut self, cancel: bool) {
        self.inner.cancelled = cancel;
    }

    pub fn get_message(&self) -> &str {
        &self.inner.message
    }

    pub fn set_message(&mut self, message: String) {
        self.inner.message = message;
    }
}

#[derive(Debug, Clone)]
pub struct PlayerSendPacketEvent {
    pub player_name: String,
    pub packet: Clientbound,
    pub cancelled: bool,
}

impl PlayerSendPacketEvent {
    pub fn new(player_name: String, packet: Clientbound) -> Self {
        Self {
            player_name,
            packet,
            cancelled: false,
        }
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancelled
    }

    pub fn set_cancelled(&mut self, cancel: bool) {
        self.cancelled = cancel;
    }

    pub fn get_packet(&self) -> &Clientbound {
        &self.packet
    }
}

impl Default for PluginManager {
    fn default() -> Self {
        Self::new()
    }
}

pub struct PluginManager;

impl PluginManager {
    pub fn new() -> Self {
        Self
    }

    pub fn call_chat_event(&self, event: &mut PlayerChatEvent) {
        let _ = event;
    }

    pub fn call_command_preprocess_event(&self, event: &mut PlayerCommandPreprocessEvent) {
        let _ = event;
    }

    pub fn call_send_packet_event(&self, event: &mut PlayerSendPacketEvent) {
        let _ = event;
    }
}
