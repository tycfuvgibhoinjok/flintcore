use std::collections::HashMap;

use crate::gameplay::{blocks, items};

#[derive(Debug, Clone, PartialEq)]
pub enum CommandAction {
    Give {
        target: String,
        item_id: i16,
        amount: u8,
        damage: i16,
    },
    TeleportToPlayer {
        victim: String,
        target: String,
    },
    TeleportToCoords {
        victim: String,
        x: f64,
        y: f64,
        z: f64,
    },
    SetTime(i64),
    AddTime(i64),
    Kill(String),
    Kick {
        target: String,
        reason: String,
    },
    Ban(String),
    Pardon(String),
    BanIp(String),
    PardonIp(String),
    Op(String),
    Deop(String),
    WhitelistOn,
    WhitelistOff,
    WhitelistAdd(String),
    WhitelistRemove(String),
    WhitelistReload,
    SaveAll,
    SaveOn,
    SaveOff,
    Stop,
    Reload,
}

#[derive(Debug, Clone, Default)]
pub struct CommandContext {
    pub online_players: Vec<String>,
    pub whitelisted_players: Vec<String>,
    pub tps_records: Vec<f64>,
    pub plugins: Vec<(String, String, bool)>,
}

#[derive(Debug, Default)]
pub struct CommandResult {
    pub sender_messages: Vec<String>,
    pub broadcast_messages: Vec<String>,
    pub target_messages: Vec<(String, String)>,
    pub actions: Vec<CommandAction>,
    pub handled: bool,
}

impl CommandResult {
    pub fn unhandled() -> Self {
        Self {
            sender_messages: Vec::new(),
            broadcast_messages: Vec::new(),
            target_messages: Vec::new(),
            actions: Vec::new(),
            handled: false,
        }
    }

    pub fn handled() -> Self {
        Self {
            sender_messages: Vec::new(),
            broadcast_messages: Vec::new(),
            target_messages: Vec::new(),
            actions: Vec::new(),
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

    pub fn with_target_message(mut self, target: String, msg: String) -> Self {
        self.target_messages.push((target, msg));
        self.handled = true;
        self
    }

    pub fn with_action(mut self, action: CommandAction) -> Self {
        self.actions.push(action);
        self.handled = true;
        self
    }
}

pub trait Command {
    fn name(&self) -> &str;
    fn aliases(&self) -> &[&str] {
        &[]
    }
    fn description(&self) -> &str {
        ""
    }
    fn usage(&self) -> &str {
        ""
    }
    fn permission(&self) -> &str {
        ""
    }
    fn execute(&self, sender: &str, args: &[&str], ctx: &CommandContext) -> CommandResult;
}

pub struct CommandMap {
    commands: HashMap<String, Box<dyn Command + Send>>,
    alias_to_name: HashMap<String, String>,
}

impl CommandMap {
    pub fn new() -> Self {
        let mut map = Self {
            commands: HashMap::new(),
            alias_to_name: HashMap::new(),
        };
        map.register_defaults();
        map
    }

    fn register<C: Command + Send + 'static>(&mut self, cmd: C) {
        let primary = cmd.name().to_lowercase();
        for &alias in cmd.aliases() {
            self.alias_to_name
                .insert(alias.to_lowercase(), primary.clone());
        }
        self.commands.insert(primary, Box::new(cmd));
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
        self.register(TpsCommand);
        self.register(PoseidonCommand);
    }

    pub fn get_command(&self, label: &str) -> Option<&(dyn Command + Send)> {
        let lower = label.to_lowercase();
        let lower = lower.strip_prefix("bukkit:").unwrap_or(&lower);
        let lower = lower.strip_prefix("poseidon:").unwrap_or(lower);
        if let Some(cmd) = self.commands.get(lower) {
            return Some(cmd.as_ref());
        }
        if let Some(primary) = self.alias_to_name.get(lower) {
            return self.commands.get(primary).map(|c| c.as_ref());
        }
        None
    }

    pub fn dispatch(&self, sender: &str, command_line: &str) -> CommandResult {
        self.dispatch_with_context(sender, command_line, &CommandContext::default())
    }

    pub fn dispatch_with_context(
        &self,
        sender: &str,
        command_line: &str,
        ctx: &CommandContext,
    ) -> CommandResult {
        let mut parts = command_line.split_whitespace();
        let label = parts.next().unwrap_or("");
        let args: Vec<&str> = parts.collect();

        if (label.eq_ignore_ascii_case("poseidon")
            || label.eq_ignore_ascii_case("projectposeidon")
            || label.eq_ignore_ascii_case("poseidon:poseidon"))
            && args
                .first()
                .is_some_and(|a| a.eq_ignore_ascii_case("resolve"))
        {
            return self.execute_resolve(&args[1..]);
        }

        if let Some(cmd) = self.get_command(label) {
            cmd.execute(sender, &args, ctx)
        } else {
            CommandResult::unhandled()
        }
    }

    fn execute_resolve(&self, args: &[&str]) -> CommandResult {
        if args.is_empty() {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /poseidon resolve <command>".to_string());
        }
        let target = args[0].trim_start_matches('/').to_lowercase();
        if let Some(cmd) = self.get_command(&target) {
            let mut res = CommandResult::handled();
            res.sender_messages
                .push(format!("\u{00A7}aCommand: \u{00A7}f/{target}"));
            res.sender_messages
                .push("\u{00A7}bSource: \u{00A7}fServer (Poseidon/Bukkit)".to_string());
            if !cmd.aliases().is_empty() {
                res.sender_messages.push(format!(
                    "\u{00A7}aAliases: \u{00A7}f{}",
                    cmd.aliases().join(", ")
                ));
            }
            if !cmd.description().is_empty() {
                res.sender_messages.push(format!(
                    "\u{00A7}aDescription: \u{00A7}f{}",
                    cmd.description()
                ));
            }
            if !cmd.permission().is_empty() {
                res.sender_messages.push(format!(
                    "\u{00A7}aPermission: \u{00A7}f{}",
                    cmd.permission()
                ));
            }
            res
        } else {
            CommandResult::handled()
                .with_sender_message(format!("\u{00A7}cCommand '{target}' not found."))
                .with_sender_message(
                    "\u{00A7}eNote: The command might not be registered or might be case-sensitive."
                        .to_string(),
                )
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
    fn aliases(&self) -> &[&str] {
        &["?"]
    }
    fn description(&self) -> &str {
        "Shows the help menu"
    }
    fn usage(&self) -> &str {
        "/help"
    }
    fn permission(&self) -> &str {
        "bukkit.command.help"
    }

    fn execute(&self, _sender: &str, _args: &[&str], _ctx: &CommandContext) -> CommandResult {
        let mut result = CommandResult::handled();
        result
            .sender_messages
            .push("To run the server without a gui, start it like this:".to_string());
        result
            .sender_messages
            .push("   java -Xmx1024M -Xms1024M -jar minecraft_server.jar nogui".to_string());
        result.sender_messages.push("Console commands:".to_string());
        result
            .sender_messages
            .push("   help  or  ?               shows this message".to_string());
        result
            .sender_messages
            .push("   kick <player>             removes a player from the server".to_string());
        result
            .sender_messages
            .push("   ban <player>              bans a player from the server".to_string());
        result.sender_messages.push(
            "   pardon <player>           pardons a banned player so that they can connect again"
                .to_string(),
        );
        result
            .sender_messages
            .push("   ban-ip <ip>               bans an IP address from the server".to_string());
        result.sender_messages.push(
            "   pardon-ip <ip>            pardons a banned IP address so that they can connect again".to_string(),
        );
        result
            .sender_messages
            .push("   op <player>               turns a player into an op".to_string());
        result
            .sender_messages
            .push("   deop <player>             removes op status from a player".to_string());
        result.sender_messages.push(
            "   tp <player1> <player2>    moves one player to the same location as another player"
                .to_string(),
        );
        result
            .sender_messages
            .push("   give <player> <id> [num]  gives a player a resource".to_string());
        result
            .sender_messages
            .push("   tell <player> <message>   sends a private message to a player".to_string());
        result
            .sender_messages
            .push("   stop                      gracefully stops the server".to_string());
        result
            .sender_messages
            .push("   save-all                  forces a server-wide level save".to_string());
        result.sender_messages.push(
            "   save-off                  disables terrain saving (useful for backup scripts)"
                .to_string(),
        );
        result
            .sender_messages
            .push("   save-on                   re-enables terrain saving".to_string());
        result
            .sender_messages
            .push("   list                      lists all currently connected players".to_string());
        result
            .sender_messages
            .push("   say <message>             broadcasts a message to all players".to_string());
        result.sender_messages.push(
            "   time <add|set> <amount>   adds to or sets the world time (0-24000)".to_string(),
        );
        result
    }
}

pub struct SayCommand;
impl Command for SayCommand {
    fn name(&self) -> &str {
        "say"
    }
    fn description(&self) -> &str {
        "Broadcasts the given message as the console"
    }
    fn usage(&self) -> &str {
        "/say <message>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.say"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.is_empty() {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /say <message>".to_string());
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
    fn description(&self) -> &str {
        "Stops the server"
    }
    fn usage(&self) -> &str {
        "/stop"
    }
    fn permission(&self) -> &str {
        "bukkit.command.stop"
    }

    fn execute(&self, _sender: &str, _args: &[&str], _ctx: &CommandContext) -> CommandResult {
        CommandResult::handled()
            .with_broadcast("Starting Server Shutdown, Saving Data.".to_string())
            .with_broadcast("Stopping the server..".to_string())
            .with_action(CommandAction::Stop)
    }
}

pub struct ListCommand;
impl Command for ListCommand {
    fn name(&self) -> &str {
        "list"
    }
    fn description(&self) -> &str {
        "Lists all online players"
    }
    fn usage(&self) -> &str {
        "/list"
    }
    fn permission(&self) -> &str {
        "bukkit.command.list"
    }

    fn execute(&self, _sender: &str, _args: &[&str], ctx: &CommandContext) -> CommandResult {
        let players = ctx.online_players.join(", ");
        CommandResult::handled().with_sender_message(format!("Connected players: {players}"))
    }
}

pub struct SaveAllCommand;
impl Command for SaveAllCommand {
    fn name(&self) -> &str {
        "save-all"
    }
    fn description(&self) -> &str {
        "Saves the server to disk"
    }
    fn usage(&self) -> &str {
        "/save-all"
    }
    fn permission(&self) -> &str {
        "bukkit.command.save.perform"
    }

    fn execute(&self, _sender: &str, _args: &[&str], _ctx: &CommandContext) -> CommandResult {
        CommandResult::handled()
            .with_broadcast("Forcing save..".to_string())
            .with_broadcast("Save complete.".to_string())
            .with_action(CommandAction::SaveAll)
    }
}

pub struct SaveOffCommand;
impl Command for SaveOffCommand {
    fn name(&self) -> &str {
        "save-off"
    }
    fn description(&self) -> &str {
        "Disables server autosaving"
    }
    fn usage(&self) -> &str {
        "/save-off"
    }
    fn permission(&self) -> &str {
        "bukkit.command.save.disable"
    }

    fn execute(&self, _sender: &str, _args: &[&str], _ctx: &CommandContext) -> CommandResult {
        CommandResult::handled()
            .with_broadcast("Disabling level saving..".to_string())
            .with_action(CommandAction::SaveOff)
    }
}

pub struct SaveOnCommand;
impl Command for SaveOnCommand {
    fn name(&self) -> &str {
        "save-on"
    }
    fn description(&self) -> &str {
        "Enables server autosaving"
    }
    fn usage(&self) -> &str {
        "/save-on"
    }
    fn permission(&self) -> &str {
        "bukkit.command.save.enable"
    }

    fn execute(&self, _sender: &str, _args: &[&str], _ctx: &CommandContext) -> CommandResult {
        CommandResult::handled()
            .with_broadcast("Enabling level saving..".to_string())
            .with_action(CommandAction::SaveOn)
    }
}

pub struct OpCommand;
impl Command for OpCommand {
    fn name(&self) -> &str {
        "op"
    }
    fn description(&self) -> &str {
        "Gives the specified player operator status"
    }
    fn usage(&self) -> &str {
        "/op <player>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.op.give"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.len() != 1 {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /op <player>".to_string());
        }
        let target = args[0];
        CommandResult::handled()
            .with_broadcast(format!("Oping {target}"))
            .with_target_message(target.to_string(), "\u{00A7}eYou are now op!".to_string())
            .with_action(CommandAction::Op(target.to_string()))
    }
}

pub struct DeopCommand;
impl Command for DeopCommand {
    fn name(&self) -> &str {
        "deop"
    }
    fn description(&self) -> &str {
        "Takes the specified player's operator status"
    }
    fn usage(&self) -> &str {
        "/deop <player>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.op.take"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.len() != 1 {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /deop <player>".to_string());
        }
        let target = args[0];
        CommandResult::handled()
            .with_broadcast(format!("De-opping {target}"))
            .with_target_message(
                target.to_string(),
                "\u{00A7}eYou are no longer op!".to_string(),
            )
            .with_action(CommandAction::Deop(target.to_string()))
    }
}

pub struct BanCommand;
impl Command for BanCommand {
    fn name(&self) -> &str {
        "ban"
    }
    fn description(&self) -> &str {
        "Prevents the specified player from using this server"
    }
    fn usage(&self) -> &str {
        "/ban <player>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.ban.player"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.len() != 1 {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /ban <player>".to_string());
        }
        CommandResult::handled()
            .with_broadcast(format!("Banning {}", args[0]))
            .with_action(CommandAction::Ban(args[0].to_string()))
    }
}

pub struct PardonCommand;
impl Command for PardonCommand {
    fn name(&self) -> &str {
        "pardon"
    }
    fn description(&self) -> &str {
        "Allows the specified player to use this server"
    }
    fn usage(&self) -> &str {
        "/pardon <player>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.unban.player"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.len() != 1 {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /pardon <player>".to_string());
        }
        CommandResult::handled()
            .with_broadcast(format!("Pardoning {}", args[0]))
            .with_action(CommandAction::Pardon(args[0].to_string()))
    }
}

pub struct BanIpCommand;
impl Command for BanIpCommand {
    fn name(&self) -> &str {
        "ban-ip"
    }
    fn description(&self) -> &str {
        "Prevents the specified IP address from using this server"
    }
    fn usage(&self) -> &str {
        "/ban-ip <address>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.ban.ip"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.len() != 1 {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /ban-ip <address>".to_string());
        }
        CommandResult::handled()
            .with_broadcast(format!("Banning ip {}", args[0]))
            .with_action(CommandAction::BanIp(args[0].to_string()))
    }
}

pub struct PardonIpCommand;
impl Command for PardonIpCommand {
    fn name(&self) -> &str {
        "pardon-ip"
    }
    fn description(&self) -> &str {
        "Allows the specified IP address to use this server"
    }
    fn usage(&self) -> &str {
        "/pardon-ip <address>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.unban.ip"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.len() != 1 {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /pardon-ip <address>".to_string());
        }
        CommandResult::handled()
            .with_broadcast(format!("Pardoning ip {}", args[0]))
            .with_action(CommandAction::PardonIp(args[0].to_string()))
    }
}

pub struct KickCommand;
impl Command for KickCommand {
    fn name(&self) -> &str {
        "kick"
    }
    fn description(&self) -> &str {
        "Removes the specified player from the server"
    }
    fn usage(&self) -> &str {
        "/kick <player>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.kick"
    }

    fn execute(&self, _sender: &str, args: &[&str], ctx: &CommandContext) -> CommandResult {
        if args.is_empty() {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /kick <player>".to_string());
        }
        let target = args[0];
        let found = ctx
            .online_players
            .iter()
            .find(|p| p.eq_ignore_ascii_case(target))
            .cloned();
        if !ctx.online_players.is_empty() && found.is_none() {
            return CommandResult::handled()
                .with_sender_message(format!("Can't find user {target}. No kick."));
        }
        let real_name = found.unwrap_or_else(|| target.to_string());
        let reason = if args.len() > 1 {
            args[1..].join(" ")
        } else {
            "Kicked by admin".to_string()
        };
        CommandResult::handled()
            .with_broadcast(format!("Kicking {real_name}"))
            .with_action(CommandAction::Kick {
                target: real_name,
                reason,
            })
    }
}

pub struct TeleportCommand;
impl Command for TeleportCommand {
    fn name(&self) -> &str {
        "tp"
    }
    fn description(&self) -> &str {
        "Teleports the given player to another player"
    }
    fn usage(&self) -> &str {
        "/tp <player> <target>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.teleport"
    }

    fn execute(&self, sender: &str, args: &[&str], ctx: &CommandContext) -> CommandResult {
        if args.len() == 3 {
            if let (Ok(x), Ok(y), Ok(z)) = (
                args[0].parse::<f64>(),
                args[1].parse::<f64>(),
                args[2].parse::<f64>(),
            ) {
                return CommandResult::handled()
                    .with_broadcast(format!("Teleporting {sender} to {x:.1}, {y:.1}, {z:.1}"))
                    .with_action(CommandAction::TeleportToCoords {
                        victim: sender.to_string(),
                        x,
                        y,
                        z,
                    });
            }
        }
        if args.len() != 2 {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /tp <player> <target>".to_string());
        }
        let victim = args[0];
        let target = args[1];
        if !ctx.online_players.is_empty() {
            let v_found = ctx
                .online_players
                .iter()
                .find(|p| p.eq_ignore_ascii_case(victim));
            let t_found = ctx
                .online_players
                .iter()
                .find(|p| p.eq_ignore_ascii_case(target));
            if v_found.is_none() {
                return CommandResult::handled()
                    .with_sender_message(format!("Can't find user {victim}. No tp."));
            }
            if t_found.is_none() {
                return CommandResult::handled()
                    .with_sender_message(format!("Can't find user {target}. No tp."));
            }
        }
        CommandResult::handled()
            .with_broadcast(format!("Teleporting {victim} to {target}"))
            .with_action(CommandAction::TeleportToPlayer {
                victim: victim.to_string(),
                target: target.to_string(),
            })
    }
}

pub struct GiveCommand;
impl Command for GiveCommand {
    fn name(&self) -> &str {
        "give"
    }
    fn description(&self) -> &str {
        "Gives the specified player a certain amount of items"
    }
    fn usage(&self) -> &str {
        "/give <player> <item> [amount]"
    }
    fn permission(&self) -> &str {
        "bukkit.command.give"
    }

    fn execute(&self, _sender: &str, args: &[&str], ctx: &CommandContext) -> CommandResult {
        // The reference ignores the wrong number of the arguments silently.
        if args.len() < 2 || args.len() > 3 {
            return CommandResult::handled();
        }
        let target = args[0];
        if !ctx.online_players.is_empty()
            && !ctx
                .online_players
                .iter()
                .any(|p| p.eq_ignore_ascii_case(target))
        {
            return CommandResult::handled()
                .with_sender_message(format!("Can't find user {target}"));
        }

        // The item is given by its number only.
        let Ok(item_id) = args[1].parse::<i16>() else {
            return CommandResult::handled()
                .with_sender_message(format!("There's no item with id {}", args[1]));
        };
        if material_name_by_id(item_id).is_none() {
            return CommandResult::handled()
                .with_sender_message(format!("There's no item with id {item_id}"));
        }

        let amount = args
            .get(2)
            .and_then(|s| s.parse::<i32>().ok())
            .unwrap_or(1)
            .clamp(1, 64) as u8;

        CommandResult::handled()
            .with_broadcast(format!("Giving {target} some {item_id}"))
            .with_action(CommandAction::Give {
                target: target.to_string(),
                item_id,
                amount,
                damage: 0,
            })
    }
}

pub struct TimeCommand;
impl Command for TimeCommand {
    fn name(&self) -> &str {
        "time"
    }
    fn description(&self) -> &str {
        "Changes the time on each world"
    }
    fn usage(&self) -> &str {
        "/time set <value>\n/time add <value>"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.is_empty() {
            return CommandResult::handled().with_sender_message(
                "\u{00A7}cIncorrect usage. Correct usage:\n/time set <value>\n/time add <value>"
                    .to_string(),
            );
        }
        if args.len() == 1 {
            let val = match args[0].to_lowercase().as_str() {
                "day" => Some(0i64),
                "night" => Some(12500i64),
                other => other.parse::<i64>().ok(),
            };
            if let Some(v) = val {
                return CommandResult::handled()
                    .with_broadcast(format!("Set time to {v}"))
                    .with_action(CommandAction::SetTime(v));
            }
            return CommandResult::handled()
                .with_sender_message(format!("Unable to convert time value, {}", args[0]));
        }

        let val = match args[1].to_lowercase().as_str() {
            "day" => Some(0i64),
            "night" => Some(12500i64),
            other => other.parse::<i64>().ok(),
        };
        let Some(v) = val else {
            return CommandResult::handled()
                .with_sender_message(format!("Unable to convert time value, {}", args[1]));
        };

        if args[0].eq_ignore_ascii_case("add") {
            CommandResult::handled()
                .with_broadcast(format!("Added {v} to time"))
                .with_action(CommandAction::AddTime(v))
        } else if args[0].eq_ignore_ascii_case("set") {
            CommandResult::handled()
                .with_broadcast(format!("Set time to {v}"))
                .with_action(CommandAction::SetTime(v))
        } else {
            CommandResult::handled()
                .with_sender_message("Unknown method, use either \"add\" or \"set\"".to_string())
        }
    }
}

pub struct WhitelistCommand;
impl Command for WhitelistCommand {
    fn name(&self) -> &str {
        "whitelist"
    }
    fn description(&self) -> &str {
        "Prevents the specified player from using this server"
    }
    fn usage(&self) -> &str {
        "/whitelist (add|remove) <player>\n/whitelist (on|off|list|reload)"
    }

    fn execute(&self, _sender: &str, args: &[&str], ctx: &CommandContext) -> CommandResult {
        if args.len() == 1 {
            match args[0].to_lowercase().as_str() {
                "reload" => {
                    return CommandResult::handled()
                        .with_broadcast("Reloaded white-list from file".to_string())
                        .with_action(CommandAction::WhitelistReload);
                }
                "on" => {
                    return CommandResult::handled()
                        .with_broadcast("Turned on white-listing".to_string())
                        .with_action(CommandAction::WhitelistOn);
                }
                "off" => {
                    return CommandResult::handled()
                        .with_broadcast("Turned off white-listing".to_string())
                        .with_action(CommandAction::WhitelistOff);
                }
                "list" => {
                    let list = ctx.whitelisted_players.join(" ");
                    return CommandResult::handled()
                        .with_sender_message(format!("White-listed players: {list}"));
                }
                _ => {}
            }
        } else if args.len() == 2 {
            match args[0].to_lowercase().as_str() {
                "add" => {
                    return CommandResult::handled()
                        .with_broadcast(format!("Added {} to white-list", args[1]))
                        .with_action(CommandAction::WhitelistAdd(args[1].to_string()));
                }
                "remove" => {
                    return CommandResult::handled()
                        .with_broadcast(format!("Removed {} from white-list", args[1]))
                        .with_action(CommandAction::WhitelistRemove(args[1].to_string()));
                }
                _ => {}
            }
        }
        CommandResult::handled()
            .with_sender_message(format!("\u{00A7}cCorrect command usage:\n{}", self.usage()))
    }
}

pub struct TellCommand;
impl Command for TellCommand {
    fn name(&self) -> &str {
        "tell"
    }
    fn description(&self) -> &str {
        "Sends a private message to the given player"
    }
    fn usage(&self) -> &str {
        "/tell <player> <message>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.tell"
    }

    fn execute(&self, sender: &str, args: &[&str], ctx: &CommandContext) -> CommandResult {
        if args.len() < 2 {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /tell <player> <message>".to_string());
        }
        let target = args[0];
        if !ctx.online_players.is_empty()
            && !ctx
                .online_players
                .iter()
                .any(|p| p.eq_ignore_ascii_case(target))
        {
            return CommandResult::handled()
                .with_sender_message("There's no player by that name online.".to_string());
        }
        let message = args[1..].join(" ");
        let whisper = format!("\u{00A7}7{sender} whispers {message}");
        CommandResult::handled()
            .with_sender_message(whisper.clone())
            .with_target_message(target.to_string(), whisper)
    }
}

pub struct MeCommand;
impl Command for MeCommand {
    fn name(&self) -> &str {
        "me"
    }
    fn description(&self) -> &str {
        "Performs the specified action in chat"
    }
    fn usage(&self) -> &str {
        "/me <action>"
    }
    fn permission(&self) -> &str {
        "bukkit.command.me"
    }

    fn execute(&self, sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.is_empty() {
            return CommandResult::handled()
                .with_sender_message("\u{00A7}cUsage: /me <action>".to_string());
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
    fn description(&self) -> &str {
        "Commits suicide, only usable as a player"
    }
    fn usage(&self) -> &str {
        "/kill"
    }
    fn permission(&self) -> &str {
        "bukkit.command.kill"
    }

    fn execute(&self, sender: &str, _args: &[&str], _ctx: &CommandContext) -> CommandResult {
        CommandResult::handled().with_action(CommandAction::Kill(sender.to_string()))
    }
}

pub struct VersionCommand;
impl Command for VersionCommand {
    fn name(&self) -> &str {
        "version"
    }
    fn aliases(&self) -> &[&str] {
        &["ver", "about"]
    }
    fn description(&self) -> &str {
        "Gets the version of this server including any plugins in use"
    }
    fn usage(&self) -> &str {
        "/version [plugin name]"
    }
    fn permission(&self) -> &str {
        "bukkit.command.version"
    }

    fn execute(&self, _sender: &str, args: &[&str], ctx: &CommandContext) -> CommandResult {
        if args.is_empty() {
            CommandResult::handled()
                .with_sender_message(
                    "\u{00A7}7This server is running \u{00A7}bflintcore (Project Poseidon CB1060)"
                        .to_string(),
                )
                .with_sender_message(
                    "\u{00A7}7Version: \u{00A7}c0.1.0 (MC: Beta 1.7.3)".to_string(),
                )
        } else {
            let name = args.join(" ");
            if let Some((pname, pver, _)) = ctx
                .plugins
                .iter()
                .find(|(n, _, _)| n.eq_ignore_ascii_case(&name))
            {
                CommandResult::handled().with_sender_message(format!(
                    "\u{00A7}a{pname}\u{00A7}f version \u{00A7}a{pver}"
                ))
            } else {
                CommandResult::handled()
                    .with_sender_message(
                        "This server is not running any plugin by that name.".to_string(),
                    )
                    .with_sender_message("Use /plugins to get a list of plugins.".to_string())
            }
        }
    }
}

pub struct ReloadCommand;
impl Command for ReloadCommand {
    fn name(&self) -> &str {
        "reload"
    }
    fn aliases(&self) -> &[&str] {
        &["rl"]
    }
    fn description(&self) -> &str {
        "Reloads the server configuration and plugins"
    }
    fn usage(&self) -> &str {
        "/reload"
    }
    fn permission(&self) -> &str {
        "bukkit.command.reload"
    }

    fn execute(&self, _sender: &str, _args: &[&str], _ctx: &CommandContext) -> CommandResult {
        CommandResult::handled()
            .with_sender_message("\u{00A7}aReload complete.".to_string())
            .with_action(CommandAction::Reload)
    }
}

pub struct PluginsCommand;
impl Command for PluginsCommand {
    fn name(&self) -> &str {
        "plugins"
    }
    fn aliases(&self) -> &[&str] {
        &["pl"]
    }
    fn description(&self) -> &str {
        "Gets a list of plugins running on the server"
    }
    fn usage(&self) -> &str {
        "/plugins"
    }
    fn permission(&self) -> &str {
        "bukkit.command.plugins"
    }

    fn execute(&self, _sender: &str, _args: &[&str], ctx: &CommandContext) -> CommandResult {
        let total = ctx.plugins.len();
        let enabled = ctx.plugins.iter().filter(|(_, _, en)| *en).count();
        let mut list = String::new();
        for (i, (name, _, en)) in ctx.plugins.iter().enumerate() {
            if i > 0 {
                list.push_str("\u{00A7}f, ");
            }
            list.push_str(if *en { "\u{00A7}a" } else { "\u{00A7}c" });
            list.push_str(name);
        }
        CommandResult::handled().with_sender_message(format!("Plugins ({enabled}/{total}): {list}"))
    }
}

pub struct TpsCommand;
impl Command for TpsCommand {
    fn name(&self) -> &str {
        "tps"
    }
    fn description(&self) -> &str {
        "Shows the server's TPS for various intervals"
    }
    fn usage(&self) -> &str {
        "/tps"
    }
    fn permission(&self) -> &str {
        "poseidon.command.tps"
    }

    fn execute(&self, _sender: &str, _args: &[&str], ctx: &CommandContext) -> CommandResult {
        let intervals = [
            ("5s", 5usize),
            ("30s", 30),
            ("1m", 60),
            ("5m", 300),
            ("10m", 600),
            ("15m", 900),
        ];
        let mut parts = Vec::with_capacity(intervals.len());
        for (label, secs) in intervals {
            let count = ctx.tps_records.len().min(secs);
            let avg = if count == 0 {
                20.0
            } else {
                ctx.tps_records[..count].iter().sum::<f64>() / (count as f64)
            };
            let color = if avg >= 19.0 {
                "\u{00A7}a"
            } else if avg >= 15.0 {
                "\u{00A7}e"
            } else {
                "\u{00A7}c"
            };
            parts.push(format!("{color}{avg:.2}\u{00A7}b ({label})"));
        }
        CommandResult::handled()
            .with_sender_message(format!("\u{00A7}bServer TPS: {}", parts.join(", ")))
    }
}

pub struct PoseidonCommand;
impl Command for PoseidonCommand {
    fn name(&self) -> &str {
        "poseidon"
    }
    fn aliases(&self) -> &[&str] {
        &["projectposeidon"]
    }
    fn description(&self) -> &str {
        "Show data regarding the server's version of Project Poseidon"
    }
    fn usage(&self) -> &str {
        "/poseidon"
    }

    fn execute(&self, _sender: &str, args: &[&str], _ctx: &CommandContext) -> CommandResult {
        if args.is_empty() {
            CommandResult::handled()
                .with_sender_message(
                    "\u{00A7}7This server is running \u{00A7}bflintcore (Project Poseidon)\u{00A7}7:"
                        .to_string(),
                )
                .with_sender_message("\u{00A7}7 - Version: \u{00A7}e0.1.0-CB1060".to_string())
                .with_sender_message("\u{00A7}aThis is a release build.".to_string())
        } else if args[0].eq_ignore_ascii_case("uuid") {
            if args.len() < 2 {
                CommandResult::handled().with_sender_message(
                    "\u{00A7}7Please specify a user /poseidon uuid (username)".to_string(),
                )
            } else {
                let user = args[1];
                let uuid = offline_uuid(user);
                CommandResult::handled()
                    .with_sender_message(format!("\u{00A7}7Username: {user}"))
                    .with_sender_message(format!("\u{00A7}7UUID: {uuid}"))
                    .with_sender_message("\u{00A7}7UUID Type: \u{00A7}cOffline".to_string())
            }
        } else {
            CommandResult::handled()
                .with_sender_message("\u{00A7}7Unknown sub command.".to_string())
        }
    }
}

pub fn offline_uuid(username: &str) -> String {
    // Deterministic UUID v3 format from "OfflinePlayer:<username>"
    let input = format!("OfflinePlayer:{username}");
    let mut h1: u64 = 0xcbf29ce484222325;
    let mut h2: u64 = 0x84222325cbf29ce4;
    for &b in input.as_bytes() {
        h1 ^= b as u64;
        h1 = h1.wrapping_mul(0x100000001b3);
        h2 = h2.wrapping_add(b as u64).wrapping_mul(0x100000001b3);
    }
    let mut bytes = [0u8; 16];
    bytes[..8].copy_from_slice(&h1.to_be_bytes());
    bytes[8..].copy_from_slice(&h2.to_be_bytes());
    bytes[6] = (bytes[6] & 0x0f) | 0x30; // version 3
    bytes[8] = (bytes[8] & 0x3f) | 0x80; // variant RFC4122
    format!(
        "{:02x}{:02x}{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        bytes[0], bytes[1], bytes[2], bytes[3],
        bytes[4], bytes[5],
        bytes[6], bytes[7],
        bytes[8], bytes[9],
        bytes[10], bytes[11], bytes[12], bytes[13], bytes[14], bytes[15]
    )
}

/// Match item/block material by numeric ID or Bukkit `Material` name (`Material.matchMaterial`).
pub fn match_material(input: &str) -> Option<(i16, &'static str)> {
    if let Ok(id) = input.parse::<i16>() {
        return material_name_by_id(id).map(|name| (id, name));
    }
    let normalized: String = input
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .map(|c| c.to_ascii_uppercase())
        .collect();

    for &(id, name) in MATERIAL_TABLE {
        if name == normalized {
            return Some((id, name));
        }
    }
    None
}

pub fn material_name_by_id(id: i16) -> Option<&'static str> {
    MATERIAL_TABLE
        .iter()
        .find_map(|&(mid, name)| if mid == id { Some(name) } else { None })
}

const MATERIAL_TABLE: &[(i16, &str)] = &[
    (blocks::STONE as i16, "STONE"),
    (blocks::GRASS as i16, "GRASS"),
    (blocks::DIRT as i16, "DIRT"),
    (blocks::COBBLESTONE as i16, "COBBLESTONE"),
    (blocks::WOOD_PLANKS as i16, "WOOD"),
    (blocks::WOOD_PLANKS as i16, "PLANKS"),
    (blocks::SAPLING as i16, "SAPLING"),
    (blocks::BEDROCK as i16, "BEDROCK"),
    (blocks::WATER as i16, "WATER"),
    (blocks::STATIONARY_WATER as i16, "STATIONARY_WATER"),
    (blocks::LAVA as i16, "LAVA"),
    (blocks::STATIONARY_LAVA as i16, "STATIONARY_LAVA"),
    (blocks::SAND as i16, "SAND"),
    (blocks::GRAVEL as i16, "GRAVEL"),
    (blocks::GOLD_ORE as i16, "GOLD_ORE"),
    (blocks::IRON_ORE as i16, "IRON_ORE"),
    (blocks::COAL_ORE as i16, "COAL_ORE"),
    (blocks::LOG as i16, "LOG"),
    (blocks::LEAVES as i16, "LEAVES"),
    (blocks::SPONGE as i16, "SPONGE"),
    (blocks::GLASS as i16, "GLASS"),
    (blocks::LAPIS_ORE as i16, "LAPIS_ORE"),
    (blocks::LAPIS_BLOCK as i16, "LAPIS_BLOCK"),
    (blocks::DISPENSER as i16, "DISPENSER"),
    (blocks::SANDSTONE as i16, "SANDSTONE"),
    (blocks::NOTE_BLOCK as i16, "NOTE_BLOCK"),
    (blocks::BED as i16, "BED_BLOCK"),
    (blocks::POWERED_RAIL as i16, "POWERED_RAIL"),
    (blocks::DETECTOR_RAIL as i16, "DETECTOR_RAIL"),
    (blocks::STICKY_PISTON as i16, "PISTON_STICKY_BASE"),
    (blocks::WEB as i16, "WEB"),
    (blocks::TALL_GRASS as i16, "LONG_GRASS"),
    (blocks::DEAD_BUSH as i16, "DEAD_BUSH"),
    (blocks::PISTON as i16, "PISTON_BASE"),
    (blocks::PISTON_HEAD as i16, "PISTON_EXTENSION"),
    (blocks::WOOL as i16, "WOOL"),
    (blocks::YELLOW_FLOWER as i16, "YELLOW_FLOWER"),
    (blocks::RED_ROSE as i16, "RED_ROSE"),
    (blocks::BROWN_MUSHROOM as i16, "BROWN_MUSHROOM"),
    (blocks::RED_MUSHROOM as i16, "RED_MUSHROOM"),
    (blocks::GOLD_BLOCK as i16, "GOLD_BLOCK"),
    (blocks::IRON_BLOCK as i16, "IRON_BLOCK"),
    (blocks::DOUBLE_STEP as i16, "DOUBLE_STEP"),
    (blocks::STEP as i16, "STEP"),
    (blocks::BRICK as i16, "BRICK"),
    (blocks::TNT as i16, "TNT"),
    (blocks::BOOKSHELF as i16, "BOOKSHELF"),
    (blocks::MOSSY_COBBLESTONE as i16, "MOSSY_COBBLESTONE"),
    (blocks::OBSIDIAN as i16, "OBSIDIAN"),
    (blocks::TORCH as i16, "TORCH"),
    (blocks::FIRE as i16, "FIRE"),
    (blocks::MOB_SPAWNER as i16, "MOB_SPAWNER"),
    (blocks::WOOD_STAIRS as i16, "WOOD_STAIRS"),
    (blocks::CHEST as i16, "CHEST"),
    (blocks::REDSTONE_WIRE as i16, "REDSTONE_WIRE"),
    (blocks::DIAMOND_ORE as i16, "DIAMOND_ORE"),
    (blocks::DIAMOND_BLOCK as i16, "DIAMOND_BLOCK"),
    (blocks::WORKBENCH as i16, "WORKBENCH"),
    (blocks::CROPS as i16, "CROPS"),
    (blocks::FARMLAND as i16, "SOIL"),
    (blocks::FURNACE as i16, "FURNACE"),
    (blocks::LIT_FURNACE as i16, "BURNING_FURNACE"),
    (blocks::SIGN as i16, "SIGN_POST"),
    (blocks::WOODEN_DOOR as i16, "WOODEN_DOOR"),
    (blocks::LADDER as i16, "LADDER"),
    (blocks::RAIL as i16, "RAILS"),
    (blocks::COBBLESTONE_STAIRS as i16, "COBBLESTONE_STAIRS"),
    (blocks::WALL_SIGN as i16, "WALL_SIGN"),
    (blocks::LEVER as i16, "LEVER"),
    (blocks::STONE_PLATE as i16, "STONE_PLATE"),
    (blocks::IRON_DOOR as i16, "IRON_DOOR_BLOCK"),
    (blocks::WOOD_PLATE as i16, "WOOD_PLATE"),
    (blocks::REDSTONE_ORE as i16, "REDSTONE_ORE"),
    (blocks::LIT_REDSTONE_ORE as i16, "GLOWING_REDSTONE_ORE"),
    (blocks::REDSTONE_TORCH_OFF as i16, "REDSTONE_TORCH_OFF"),
    (blocks::REDSTONE_TORCH_ON as i16, "REDSTONE_TORCH_ON"),
    (blocks::STONE_BUTTON as i16, "STONE_BUTTON"),
    (blocks::SNOW as i16, "SNOW"),
    (blocks::ICE as i16, "ICE"),
    (blocks::SNOW_BLOCK as i16, "SNOW_BLOCK"),
    (blocks::CACTUS as i16, "CACTUS"),
    (blocks::CLAY as i16, "CLAY"),
    (blocks::REED as i16, "SUGAR_CANE_BLOCK"),
    (blocks::JUKEBOX as i16, "JUKEBOX"),
    (blocks::FENCE as i16, "FENCE"),
    (blocks::PUMPKIN as i16, "PUMPKIN"),
    (blocks::NETHERRACK as i16, "NETHERRACK"),
    (blocks::SOUL_SAND as i16, "SOUL_SAND"),
    (blocks::GLOWSTONE as i16, "GLOWSTONE"),
    (blocks::PORTAL as i16, "PORTAL"),
    (blocks::JACK_O_LANTERN as i16, "JACK_O_LANTERN"),
    (blocks::CAKE as i16, "CAKE_BLOCK"),
    (blocks::REPEATER_OFF as i16, "DIODE_BLOCK_OFF"),
    (blocks::REPEATER_ON as i16, "DIODE_BLOCK_ON"),
    (blocks::LOCKED_CHEST as i16, "LOCKED_CHEST"),
    (blocks::TRAPDOOR as i16, "TRAP_DOOR"),
    (items::IRON_SPADE, "IRON_SPADE"),
    (items::IRON_PICKAXE, "IRON_PICKAXE"),
    (items::IRON_AXE, "IRON_AXE"),
    (items::FLINT_AND_STEEL, "FLINT_AND_STEEL"),
    (items::APPLE, "APPLE"),
    (items::BOW, "BOW"),
    (items::ARROW, "ARROW"),
    (items::COAL, "COAL"),
    (items::DIAMOND, "DIAMOND"),
    (items::IRON_INGOT, "IRON_INGOT"),
    (items::GOLD_INGOT, "GOLD_INGOT"),
    (items::IRON_SWORD, "IRON_SWORD"),
    (items::WOOD_SWORD, "WOOD_SWORD"),
    (items::WOOD_SPADE, "WOOD_SPADE"),
    (items::WOOD_PICKAXE, "WOOD_PICKAXE"),
    (items::WOOD_AXE, "WOOD_AXE"),
    (items::STONE_SWORD, "STONE_SWORD"),
    (items::STONE_SPADE, "STONE_SPADE"),
    (items::STONE_PICKAXE, "STONE_PICKAXE"),
    (items::STONE_AXE, "STONE_AXE"),
    (items::DIAMOND_SWORD, "DIAMOND_SWORD"),
    (items::DIAMOND_SPADE, "DIAMOND_SPADE"),
    (items::DIAMOND_PICKAXE, "DIAMOND_PICKAXE"),
    (items::DIAMOND_AXE, "DIAMOND_AXE"),
    (items::STICK, "STICK"),
    (items::BOWL, "BOWL"),
    (items::MUSHROOM_SOUP, "MUSHROOM_SOUP"),
    (items::GOLD_SWORD, "GOLD_SWORD"),
    (items::GOLD_SPADE, "GOLD_SPADE"),
    (items::GOLD_PICKAXE, "GOLD_PICKAXE"),
    (items::GOLD_AXE, "GOLD_AXE"),
    (items::STRING, "STRING"),
    (items::FEATHER, "FEATHER"),
    (items::SULPHUR, "SULPHUR"),
    (items::WOOD_HOE, "WOOD_HOE"),
    (items::STONE_HOE, "STONE_HOE"),
    (items::IRON_HOE, "IRON_HOE"),
    (items::DIAMOND_HOE, "DIAMOND_HOE"),
    (items::GOLD_HOE, "GOLD_HOE"),
    (items::SEEDS, "SEEDS"),
    (items::WHEAT, "WHEAT"),
    (items::BREAD, "BREAD"),
    (items::LEATHER_HELMET, "LEATHER_HELMET"),
    (items::LEATHER_CHESTPLATE, "LEATHER_CHESTPLATE"),
    (items::LEATHER_LEGGINGS, "LEATHER_LEGGINGS"),
    (items::LEATHER_BOOTS, "LEATHER_BOOTS"),
    (items::CHAINMAIL_HELMET, "CHAINMAIL_HELMET"),
    (items::CHAINMAIL_CHESTPLATE, "CHAINMAIL_CHESTPLATE"),
    (items::CHAINMAIL_LEGGINGS, "CHAINMAIL_LEGGINGS"),
    (items::CHAINMAIL_BOOTS, "CHAINMAIL_BOOTS"),
    (items::IRON_HELMET, "IRON_HELMET"),
    (items::IRON_CHESTPLATE, "IRON_CHESTPLATE"),
    (items::IRON_LEGGINGS, "IRON_LEGGINGS"),
    (items::IRON_BOOTS, "IRON_BOOTS"),
    (items::DIAMOND_HELMET, "DIAMOND_HELMET"),
    (items::DIAMOND_CHESTPLATE, "DIAMOND_CHESTPLATE"),
    (items::DIAMOND_LEGGINGS, "DIAMOND_LEGGINGS"),
    (items::DIAMOND_BOOTS, "DIAMOND_BOOTS"),
    (items::GOLD_HELMET, "GOLD_HELMET"),
    (items::GOLD_CHESTPLATE, "GOLD_CHESTPLATE"),
    (items::GOLD_LEGGINGS, "GOLD_LEGGINGS"),
    (items::GOLD_BOOTS, "GOLD_BOOTS"),
    (items::FLINT, "FLINT"),
    (items::PORK, "PORK"),
    (items::GRILLED_PORK, "GRILLED_PORK"),
    (items::PAINTING, "PAINTING"),
    (items::GOLDEN_APPLE, "GOLDEN_APPLE"),
    (items::SIGN, "SIGN"),
    (items::WOOD_DOOR, "WOOD_DOOR"),
    (items::BUCKET, "BUCKET"),
    (items::WATER_BUCKET, "WATER_BUCKET"),
    (items::LAVA_BUCKET, "LAVA_BUCKET"),
    (items::MINECART, "MINECART"),
    (items::SADDLE, "SADDLE"),
    (items::IRON_DOOR, "IRON_DOOR"),
    (items::REDSTONE, "REDSTONE"),
    (items::SNOW_BALL, "SNOW_BALL"),
    (items::BOAT, "BOAT"),
    (items::LEATHER, "LEATHER"),
    (items::MILK_BUCKET, "MILK_BUCKET"),
    (items::CLAY_BRICK, "CLAY_BRICK"),
    (items::CLAY_BALL, "CLAY_BALL"),
    (items::SUGAR_CANE, "SUGAR_CANE"),
    (items::PAPER, "PAPER"),
    (items::BOOK, "BOOK"),
    (items::SLIME_BALL, "SLIME_BALL"),
    (items::STORAGE_MINECART, "STORAGE_MINECART"),
    (items::POWERED_MINECART, "POWERED_MINECART"),
    (items::EGG, "EGG"),
    (items::COMPASS, "COMPASS"),
    (items::FISHING_ROD, "FISHING_ROD"),
    (items::WATCH, "WATCH"),
    (items::GLOWSTONE_DUST, "GLOWSTONE_DUST"),
    (items::RAW_FISH, "RAW_FISH"),
    (items::COOKED_FISH, "COOKED_FISH"),
    (items::INK_SACK, "INK_SACK"),
    (items::BONE, "BONE"),
    (items::SUGAR, "SUGAR"),
    (items::CAKE, "CAKE"),
    (items::BED, "BED"),
    (items::DIODE, "DIODE"),
    (items::COOKIE, "COOKIE"),
    (items::MAP, "MAP"),
    (items::SHEARS, "SHEARS"),
    (items::GOLD_RECORD, "GOLD_RECORD"),
    (items::GREEN_RECORD, "GREEN_RECORD"),
];

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

        // Alias `?`
        let res_alias = map.dispatch("Player", "?");
        assert!(res_alias.handled);
        assert!(!res_alias.sender_messages.is_empty());
    }

    /// The text of the help is the text of the reference.
    ///
    /// Java `ConsoleCommandHandler.a(ICommandListener)`: three header lines
    /// and eighteen lines of the commands, every line of the commands starts
    /// with three spaces.
    #[test]
    fn the_text_of_the_help_is_the_text_of_the_reference() {
        let map = CommandMap::new();
        let result = map.dispatch("Player", "help");
        assert!(result.handled);
        let expected = [
            "To run the server without a gui, start it like this:",
            "   java -Xmx1024M -Xms1024M -jar minecraft_server.jar nogui",
            "Console commands:",
            "   help  or  ?               shows this message",
            "   kick <player>             removes a player from the server",
            "   ban <player>              bans a player from the server",
            "   pardon <player>           pardons a banned player so that they can connect again",
            "   ban-ip <ip>               bans an IP address from the server",
            "   pardon-ip <ip>            pardons a banned IP address so that they can connect again",
            "   op <player>               turns a player into an op",
            "   deop <player>             removes op status from a player",
            "   tp <player1> <player2>    moves one player to the same location as another player",
            "   give <player> <id> [num]  gives a player a resource",
            "   tell <player> <message>   sends a private message to a player",
            "   stop                      gracefully stops the server",
            "   save-all                  forces a server-wide level save",
            "   save-off                  disables terrain saving (useful for backup scripts)",
            "   save-on                   re-enables terrain saving",
            "   list                      lists all currently connected players",
            "   say <message>             broadcasts a message to all players",
            "   time <add|set> <amount>   adds to or sets the world time (0-24000)",
        ];
        assert_eq!(
            result.sender_messages,
            expected.to_vec(),
            "the text of the help"
        );
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
    fn dispatch_list_shows_connected_players() {
        let map = CommandMap::new();
        let ctx = CommandContext {
            online_players: vec!["Alice".to_string(), "Bob".to_string()],
            ..Default::default()
        };
        let result = map.dispatch_with_context("Alice", "list", &ctx);
        assert!(result.handled);
        assert_eq!(
            result.sender_messages,
            vec!["Connected players: Alice, Bob"]
        );
    }

    #[test]
    /// The command `give` of the reference.
    ///
    /// Java `ConsoleCommandHandler`: the item is given by its number only,
    /// the name of the item is not accepted, the wrong number of the
    /// arguments is ignored silently, the message of the success is
    /// `Giving <player> some <id>` and the message of the error is
    /// `There's no item with id <id>`.
    fn dispatch_give_of_the_reference() {
        let map = CommandMap::new();
        let ctx = CommandContext {
            online_players: vec!["Hero".to_string()],
            ..Default::default()
        };
        let r1 = map.dispatch_with_context("Hero", "give Hero 333 1", &ctx);
        assert_eq!(
            r1.actions,
            vec![CommandAction::Give {
                target: "Hero".to_string(),
                item_id: items::BOAT,
                amount: 1,
                damage: 0,
            }]
        );
        assert_eq!(r1.broadcast_messages, vec!["Giving Hero some 333"]);

        // The name of the item is not accepted.
        let r2 = map.dispatch_with_context("Hero", "give Hero boat 1", &ctx);
        assert!(r2.actions.is_empty(), "the item is not given");
        assert_eq!(r2.sender_messages, vec!["There's no item with id boat"]);

        // The number of the item that does not exist.
        let r3 = map.dispatch_with_context("Hero", "give Hero 9999", &ctx);
        assert!(r3.actions.is_empty(), "the item is not given");
        assert_eq!(r3.sender_messages, vec!["There's no item with id 9999"]);

        // The wrong number of the arguments is ignored silently.
        let r4 = map.dispatch_with_context("Hero", "give Hero", &ctx);
        assert!(r4.actions.is_empty(), "the item is not given");
        assert!(r4.sender_messages.is_empty(), "the command is silent");
    }

    #[test]
    fn dispatch_tps_and_poseidon_resolve() {
        let map = CommandMap::new();
        let r_tps = map.dispatch("Hero", "tps");
        assert!(r_tps.handled);
        assert!(r_tps.sender_messages[0].contains("Server TPS:"));

        let r_res = map.dispatch("Hero", "poseidon resolve list");
        assert!(r_res.handled);
        assert!(r_res.sender_messages.iter().any(|m| m.contains("/list")));
    }
}
