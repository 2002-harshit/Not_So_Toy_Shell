use std::io::{BufRead, Write, stdout};

fn main() {
    let stdin = std::io::stdin();
    let mut stdin_handle = stdin.lock();
    let mut stdin_buffer = String::new();
    loop {
        print!("$ ");
        let flush_stdout = stdout().flush();
        if let Err(e) = flush_stdout {
            eprintln!("Error flushing stdout: {}", e);
            return;
        }
        match stdin_handle.read_line(&mut stdin_buffer) {
            Ok(0) => break,
            Ok(_) => {
                let trimmed_buffer = stdin_buffer.trim();
                if trimmed_buffer.is_empty() {
                    stdin_buffer.clear();
                    continue;
                }
                match trimmed_buffer {
                    "exit" => break,
                    _ => println!("{}: command not found", stdin_buffer.trim()),
                }
                stdin_buffer.clear();
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
            }
        }
    }
}
