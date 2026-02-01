use std::io::{BufRead, Write, stdout};

fn handle_type_command(args: &Vec<&str>) {
    for arg in args {
        if is_builtin(arg) {
            println!("{} is a shell builtin", arg);
        } else {
            println!("{}: not found", arg);
        }
    }
}

fn is_builtin(command: &str) -> bool {
    command == "exit" || command == "echo" || command == "type"
}

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
                let mut parts = trimmed_buffer.split_whitespace();
                let command = parts.next().unwrap();
                let args = parts.collect::<Vec<&str>>();
                match command {
                    "exit" => break,
                    "echo" => println!("{}", args.join(" ")),
                    "type" => match &args[..] {
                        [_, ..] => handle_type_command(&args),
                        _ => {}
                    },
                    _ => println!("{}: command not found", trimmed_buffer),
                }
                stdin_buffer.clear();
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
            }
        }
    }
}
