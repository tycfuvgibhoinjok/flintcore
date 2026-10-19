//! Server console terminal.
//!
//! Java `MinecraftServer.main` starts a daemon thread that reads the process'
//! standard input line by line and queues every line as a server command
//! (`MinecraftServer.issueCommand(String, ICommandListener)`). The queue is
//! drained at the end of every server tick by `MinecraftServer.b()`, which
//! dispatches the commands through the regular command map with the console
//! as the sender.
//!
//! The console sender is named `CONSOLE` (`MinecraftServer.getName()`) and
//! has full permissions, so operators can run commands without joining the
//! server as a player.

use std::io::BufRead;
use std::sync::mpsc;

/// Spawns the standard input reader thread.
///
/// Every non-empty line is pushed into `tx`; the server drains the queue in
/// its tick loop. When standard input reaches EOF (the server runs detached
/// from a terminal, e.g. under systemd or `docker run` without `-i`) the
/// thread logs a warning and exits — the server itself keeps running, exactly
/// like the vanilla console thread.
pub fn spawn_reader(tx: mpsc::Sender<String>) -> std::thread::JoinHandle<()> {
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        let mut handle = stdin.lock();
        let mut raw = Vec::new();
        loop {
            raw.clear();
            match handle.read_until(b'\n', &mut raw) {
                Ok(0) => {
                    crate::util::log::warn(
                        "console input closed, the server keeps running but accepts no further commands",
                    );
                    return;
                }
                Ok(_) => {}
                Err(err) => {
                    crate::util::log::warn(&format!("console input failed: {err}"));
                    return;
                }
            }
            // Terminals may hand out non-UTF-8 bytes; keep the command usable
            // instead of killing the console thread.
            let line = String::from_utf8_lossy(&raw);
            let command = line.trim();
            if command.is_empty() {
                continue;
            }
            if tx.send(command.to_string()).is_err() {
                // The server shut down and dropped the queue.
                return;
            }
        }
    })
}

/// Removes the Minecraft formatting codes (`§x`) from a message.
///
/// The console writes to a terminal, which cannot render the section sign
/// based colour codes, so they are stripped before printing.
pub fn strip_formatting(message: &str) -> String {
    let mut out = String::with_capacity(message.len());
    let mut chars = message.chars();
    while let Some(ch) = chars.next() {
        if ch == '\u{00A7}' {
            // The section sign is followed by a single format character.
            let _ = chars.next();
            continue;
        }
        out.push(ch);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_colour_codes() {
        assert_eq!(
            strip_formatting("\u{00A7}aHello \u{00A7}cworld"),
            "Hello world"
        );
        assert_eq!(strip_formatting("plain"), "plain");
        // A trailing section sign without a format character is dropped too.
        assert_eq!(strip_formatting("text\u{00A7}"), "text");
    }
}
