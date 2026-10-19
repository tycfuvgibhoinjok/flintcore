pub fn info(message: &str) {
    println!("[INFO] {message}");
}

pub fn warn(message: &str) {
    eprintln!("[WARN] {message}");
}

/// Output of the server console terminal (`MinecraftServer.sendMessage`).
pub fn console(message: &str) {
    println!("[CONSOLE] {message}");
}
