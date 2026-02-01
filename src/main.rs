use std::io::{BufRead, Write, stdout};

fn main() {
    print!("$ ");
    stdout().flush().unwrap();
    let stdin = std::io::stdin();
    let mut stdin_handle = stdin.lock();
    let mut stdin_buffer = String::new();
    match stdin_handle.read_line(&mut stdin_buffer) {
        Ok(_) => {
            println!("{}: command not found", stdin_buffer.trim());
        }
        Err(e) => {
            eprintln!("Error reading input: {}", e);
        }
    }
}
