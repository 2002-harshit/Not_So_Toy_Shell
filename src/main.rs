use std::{
    env,
    io::{BufRead, Write, stdout},
    os::unix::fs::PermissionsExt,
    path::PathBuf,
};

use nix::{
    sys::wait::{WaitPidFlag, waitpid},
    unistd::{ForkResult, fork},
};

struct Commands;

impl Commands {
    const ECHO: &str = "echo";
    const EXIT: &str = "exit";
    const TYPE: &str = "type";
    const PWD: &str = "pwd";
}

const BUILTINS: &[&str] = &[
    Commands::ECHO,
    Commands::EXIT,
    Commands::TYPE,
    Commands::PWD,
];

fn handle_type_command(args: &[&str]) {
    for arg in args {
        if is_builtin(arg) {
            println!("{} is a shell builtin", arg);
        } else if let Some(full_path) = is_command_in_paths_env(*arg) {
            println!("{} is {}", arg, full_path.display());
        } else {
            println!("{}: not found", arg);
        }
    }
}

fn is_builtin(command: &str) -> bool {
    BUILTINS.contains(&command)
}

fn is_command_in_paths_env(command: &str) -> Option<PathBuf> {
    let paths_env = env::var_os("PATH").unwrap_or_default();
    for path in env::split_paths(&paths_env) {
        let full_path = path.join(command);
        if full_path.is_file()
            && full_path
                .metadata()
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        {
            return Some(full_path);
        }
    }
    None
}

fn handle_non_builtins(command: &str, args: &[&str]) {
    if let Some(full_path) = is_command_in_paths_env(command) {
        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                println!("Mf call a exec family call here");
            }
            Ok(ForkResult::Parent { child }) => {
                let w = waitpid(child, None);
            }
            Err(e) => {
                eprintln!("Command execution failed: {}(Code: {})", e.desc(), e as i32);
            }
        }
    } else {
        println!("{}: command not found", command)
    }
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
                        Commands::PWD => {}
                        _ => handle_non_builtins(command, &args),
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
