use std::{
    env,
    io::{BufRead, Write, stdout},
    os::unix::fs::PermissionsExt,
};

struct Commands;

impl Commands {
    const ECHO: &str = "echo";
    const EXIT: &str = "exit";
    const TYPE: &str = "type";
}

const BUILTINS: &[&str] = &[Commands::ECHO, Commands::EXIT, Commands::TYPE];

fn handle_type_command(args: &[&str]) {
    let paths_env = env::var_os("PATH").unwrap_or_default();
    for arg in args {
        if is_builtin(arg) {
            println!("{} is a shell builtin", arg);
        } else {
            let mut found_in_path = false;
            for path in env::split_paths(&paths_env) {
                let full_path = path.join(arg);
                if full_path.is_file()
                    && full_path
                        .metadata()
                        .map(|m| m.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                {
                    println!("{} is {}", arg, full_path.display());
                    found_in_path = true;
                    break;
                }
            }
            if !found_in_path {
                println!("{}: not found", arg);
            }
        }
    }
}

fn is_builtin(command: &str) -> bool {
    BUILTINS.contains(&command)
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
                if !trimmed_buffer.is_empty() {
                    let mut parts = trimmed_buffer.split_whitespace();
                    let command = parts.next().unwrap();
                    let args = parts.collect::<Vec<&str>>();
                    match command {
                        Commands::EXIT => break,
                        Commands::ECHO => println!("{}", args.join(" ")),
                        Commands::TYPE => handle_type_command(&args),
                        _ => println!("{}: command not found", command),
                    }
                }
                stdin_buffer.clear();
            }
            Err(e) => {
                eprintln!("Error reading input: {}", e);
            }
        }
    }
}
