fn main() {
    // Test C compilation
    match std::process::Command::new("g++").args(["--version"]).output() {
        Ok(out) => println!("g++ found: {}", String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("")),
        Err(_) => println!("g++ not found"),
    }
    match std::process::Command::new("gcc").args(["--version"]).output() {
        Ok(out) => println!("gcc found: {}", String::from_utf8_lossy(&out.stdout).lines().next().unwrap_or("")),
        Err(_) => println!("gcc not found"),
    }
}
