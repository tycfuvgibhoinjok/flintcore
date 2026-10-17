use std::collections::HashMap;

#[derive(Debug, Default)]
pub struct CommandResult {
    pub sender_messages: Vec<String>,
    pub broadcast_messages: Vec<String>,
    pub handled: bool,
}

impl CommandResult {
    pub fn unhandled() -> Self {
        Self {
            sender_messages: Vec::new(),
            broadcast_messages: Vec::new(),
            handled: false,
        }
    }

    pub fn handled() -> Self {
        Self {
            sender_messages: Vec::new(),
            broadcast_messages: Vec::new(),
            handled: true,
        }
    }

    pub fn with_sender_message(mut self, msg: String) -> Self {
        self.sender_messages.push(msg);
        self.handled = true;
        self
    }

    pub fn with_broadcast(mut self, msg: String) -> Self {
        self.broadcast_messages.push(msg);
        self.handled = true;
        self
    }
}

pub trait Command {
    fn name(&self) -> &str;
    fn execute(&self, sender: &str, args: &[&str]) -> CommandResult;
}

pub struct CommandMap {
    commands: HashMap<String, Box<dyn Command + Send>>,
}

impl CommandMap {
    pub fn new() -> Self {
        let mut map = Self {
            commands: HashMap::new(),
        };
        map.register_defaults();
        map
    }

    fn register<C: Command + Send + 'static>(&mut self, cmd: C) {
        self.commands.insert(cmd.name().to_string(), Box::new(cmd));
    }

    fn register_defaults(&mut self) {
        self.register(HelpCommand);
        self.register(SayCommand);
        self.register(StopCommand);
        self.register(ListCommand);
        self.register(SaveAllCommand);
        self.register(SaveOffCommand);
        self.register(SaveOnCommand);
        self.register(OpCommand);
        self.register(DeopCommand);
        self.register(BanCommand);
        self.register(PardonCommand);
        self.register(BanIpCommand);
        self.register(PardonIpCommand);
        self.register(KickCommand);
        self.register(TeleportCommand);
        self.register(GiveCommand);
        self.register(TimeCommand);
        self.register(WhitelistCommand);
        self.register(TellCommand);
        self.register(MeCommand);
        self.register(KillCommand);
        self.register(VersionCommand);
        self.register(ReloadCommand);
        self.register(PluginsCommand);
    }

    pub fn dispatch(&self, sender: &str, command_line: &str) -> CommandResult {
        let mut parts = command_line.split_whitespace();
        let label = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();

        if let Some(cmd) = self.commands.get(label) {
            cmd.execute(sender, &args)
        } else {
            CommandResult::unhandled()
        }
    }
}

impl Default for CommandMap {
    fn default() -> Self {
        Self::new()
    }
}

pub struct HelpCommand;
impl Command for HelpCommand {
    fn name(&self) -> &str {
        "help"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        let mut result = CommandResult::handled();
        result.sender_messages.push("--- Help ---".to_string());
        result
            .sender_messages
            .push("/help - Show this help".to_string());
        result
            .sender_messages
            .push("/say <msg> - Broadcast a message".to_string());
        result
            .sender_messages
            .push("/list - List online players".to_string());
        result
            .sender_messages
            .push("/tell <player> <msg> - Private message".to_string());
        result
            .sender_messages
            .push("/me <action> - Perform an action".to_string());
        result
    }
}

pub struct SayCommand;
impl Command for SayCommand {
    fn name(&self) -> &str {
        "say"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        let message = args.join(" ");
        CommandResult::handled().with_broadcast(format!("\u{00A7}d[Server] {message}"))
    }
}

pub struct StopCommand;
impl Command for StopCommand {
    fn name(&self) -> &str {
        "stop"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled().with_broadcast("Stopping the server..".to_string())
    }
}

pub struct ListCommand;
impl Command for ListCommand {
    fn name(&self) -> &str {
        "list"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled().with_sender_message("Connected players: ".to_string())
    }
}

pub struct SaveAllCommand;
impl Command for SaveAllCommand {
    fn name(&self) -> &str {
        "save-all"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled()
            .with_broadcast("Forcing save..".to_string())
            .with_broadcast("Save complete.".to_string())
    }
}

pub struct SaveOffCommand;
impl Command for SaveOffCommand {
    fn name(&self) -> &str {
        "save-off"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled().with_broadcast("Disabling level saving..".to_string())
    }
}

pub struct SaveOnCommand;
impl Command for SaveOnCommand {
    fn name(&self) -> &str {
        "save-on"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled().with_broadcast("Enabling level saving..".to_string())
    }
}

pub struct OpCommand;
impl Command for OpCommand {
    fn name(&self) -> &str {
        "op"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        let target = args[0];
        CommandResult::handled()
            .with_broadcast(format!("Oping {target}"))
            .with_sender_message("You are now op!".to_string())
    }
}

pub struct DeopCommand;
impl Command for DeopCommand {
    fn name(&self) -> &str {
        "deop"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        let target = args[0];
        CommandResult::handled()
            .with_broadcast(format!("De-opping {target}"))
            .with_sender_message("You are no longer op!".to_string())
    }
}

pub struct BanCommand;
impl Command for BanCommand {
    fn name(&self) -> &str {
        "ban"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        CommandResult::handled().with_broadcast(format!("Banning {}", args[0]))
    }
}

pub struct PardonCommand;
impl Command for PardonCommand {
    fn name(&self) -> &str {
        "pardon"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        CommandResult::handled().with_broadcast(format!("Pardoning {}", args[0]))
    }
}

pub struct BanIpCommand;
impl Command for BanIpCommand {
    fn name(&self) -> &str {
        "ban-ip"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        CommandResult::handled().with_broadcast(format!("Banning ip {}", args[0]))
    }
}

pub struct PardonIpCommand;
impl Command for PardonIpCommand {
    fn name(&self) -> &str {
        "pardon-ip"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        CommandResult::handled().with_broadcast(format!("Pardoning ip {}", args[0]))
    }
}

pub struct KickCommand;
impl Command for KickCommand {
    fn name(&self) -> &str {
        "kick"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        CommandResult::handled().with_broadcast(format!("Kicking {}", args[0]))
    }
}

pub struct TeleportCommand;
impl Command for TeleportCommand {
    fn name(&self) -> &str {
        "tp"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::unhandled();
        }
        CommandResult::handled().with_broadcast(format!("Teleporting {} to {}", args[0], args[1]))
    }
}

pub struct GiveCommand;
impl Command for GiveCommand {
    fn name(&self) -> &str {
        "give"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        CommandResult::handled().with_broadcast(format!("Giving {} some items", args[0]))
    }
}

pub struct TimeCommand;
impl Command for TimeCommand {
    fn name(&self) -> &str {
        "time"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        CommandResult::handled().with_broadcast(format!("Set time to {}", args[0]))
    }
}

pub struct WhitelistCommand;
impl Command for WhitelistCommand {
    fn name(&self) -> &str {
        "whitelist"
    }

    fn execute(&self, _sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        match args[0] {
            "on" => CommandResult::handled().with_broadcast("Turned on white-listing".to_string()),
            "off" => {
                CommandResult::handled().with_broadcast("Turned off white-listing".to_string())
            }
            "add" if args.len() > 1 => {
                CommandResult::handled().with_broadcast(format!("Added {} to white-list", args[1]))
            }
            "remove" if args.len() > 1 => CommandResult::handled()
                .with_broadcast(format!("Removed {} from white-list", args[1])),
            _ => CommandResult::unhandled(),
        }
    }
}

pub struct TellCommand;
impl Command for TellCommand {
    fn name(&self) -> &str {
        "tell"
    }

    fn execute(&self, sender: &str, args: &[&str]) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::unhandled();
        }
        let target = args[0];
        let message = args[1..].join(" ");
        CommandResult::handled()
            .with_sender_message(format!("{sender} whispers {message}"))
            .with_broadcast(format!("[{sender}->{target}] {message}"))
    }
}

pub struct MeCommand;
impl Command for MeCommand {
    fn name(&self) -> &str {
        "me"
    }

    fn execute(&self, sender: &str, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::unhandled();
        }
        let action = args.join(" ");
        CommandResult::handled().with_broadcast(format!("* {sender} {action}"))
    }
}

pub struct KillCommand;
impl Command for KillCommand {
    fn name(&self) -> &str {
        "kill"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled()
    }
}

pub struct VersionCommand;
impl Command for VersionCommand {
    fn name(&self) -> &str {
        "version"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled()
            .with_sender_message("This server is running flintcore (Beta 1.7.3)".to_string())
    }
}

pub struct ReloadCommand;
impl Command for ReloadCommand {
    fn name(&self) -> &str {
        "reload"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled().with_broadcast("Reload complete.".to_string())
    }
}

pub struct PluginsCommand;
impl Command for PluginsCommand {
    fn name(&self) -> &str {
        "plugins"
    }

    fn execute(&self, _sender: &str, _args: &[&str]) -> CommandResult {
        CommandResult::handled().with_sender_message("Plugins: (none)".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatch_unknown_command() {
        let map = CommandMap::new();
        let result = map.dispatch("Player", "nonexistent");
        assert!(!result.handled);
    }

    #[test]
    fn dispatch_help() {
        let map = CommandMap::new();
        let result = map.dispatch("Player", "help");
        assert!(result.handled);
        assert!(!result.sender_messages.is_empty());
    }

    #[test]
    fn dispatch_say() {
        let map = CommandMap::new();
        let result = map.dispatch("Player", "say hello world");
        assert!(result.handled);
        assert_eq!(result.broadcast_messages.len(), 1);
        assert!(result.broadcast_messages[0].contains("hello world"));
    }

    #[test]
    fn dispatch_me() {
        let map = CommandMap::new();
        let result = map.dispatch("Player", "me dances");
        assert!(result.handled);
        assert_eq!(result.broadcast_messages.len(), 1);
        assert!(result.broadcast_messages[0].contains("* Player dances"));
    }

    #[test]
    fn dispatch_list() {
        let map = CommandMap::new();
        let result = map.dispatch("Player", "list");
        assert!(result.handled);
        assert!(!result.sender_messages.is_empty());
    }
}
