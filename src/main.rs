use std::{
    env,
    ffi::CString,
    io::{BufRead, Write, stdin, stdout},
    os::{
        fd::AsFd,
        unix::{ffi::OsStrExt, fs::PermissionsExt},
    },
    path::{Path, PathBuf},
    process::exit,
};

use nix::{
    sys::{
        signal::{SaFlags, SigAction, SigSet, Signal, sigaction},
        wait::waitpid,
    },
    unistd::{ForkResult, Pid, execv, fork, getpgrp, getpid, setpgid, tcsetpgrp},
};

struct Commands;

impl Commands {
    const ECHO: &str = "echo";
    const EXIT: &str = "exit";
    const TYPE: &str = "type";
    const PWD: &str = "pwd";
    const CD: &str = "cd";
}

const BUILTINS: [&str; 4] = [
    Commands::ECHO,
    Commands::EXIT,
    Commands::TYPE,
    Commands::PWD,
];

const HANDLED_SIGNALS: [Signal; 3] = [Signal::SIGTTOU, Signal::SIGTTIN, Signal::SIGINT];

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

fn handle_cd_command(path: &str) {
    let path = Path::new(path);
    if !path.is_dir() {
        println!("cd: {}: No such file or directory", path.display());
    }
    if path.is_absolute() {
        match env::set_current_dir(path) {
            Ok(_) => {}
            Err(_) => println!("cd: {}: No such file or directory", path.display()),
        }
    }
}

fn handle_non_builtins(command: &str, args: &[&str]) {
    if let Some(full_path) = is_command_in_paths_env(command) {
        match unsafe { fork() } {
            Ok(ForkResult::Child) => {
                /* resetting the signals */
                let sa = SigAction::new(
                    nix::sys::signal::SigHandler::SigDfl,
                    SaFlags::empty(),
                    SigSet::empty(),
                );
                unsafe {
                    for signal in HANDLED_SIGNALS {
                        let _ = sigaction(signal, &sa);
                    }
                }
                let pid = getpid().as_raw();
                if let Err(e) = setpgid(Pid::from_raw(0), Pid::from_raw(0)) {
                    eprintln!(
                        "Pid: {}, setpgid failed: {}(Code: {})",
                        pid,
                        e.desc(),
                        e as i32
                    );
                    exit(1);
                }
                let _ = tcsetpgrp(stdin().as_fd(), Pid::from_raw(pid));
                //* a new process group is created which has just the command and the pgid is same as pid of the child */
                let full_path = CString::new(full_path.as_os_str().as_bytes()).unwrap_or_default();
                if full_path.is_empty() {
                    eprintln!("Pid: {}, Cannot get the executible path: {}", pid, command);
                    exit(1);
                }
                let mut args_for_new_proc = Vec::with_capacity(args.len() + 1);
                args_for_new_proc.push(CString::new(command.as_bytes()).unwrap_or_default());
                for arg in args {
                    args_for_new_proc.push(CString::new(arg.as_bytes()).unwrap_or_default());
                }
                match execv(&full_path, &args_for_new_proc) {
                    Ok(_) => unreachable!(),
                    Err(e) => {
                        eprintln!(
                            "Pid: {},Command: {}: execution failed: {}(Code: {})",
                            pid,
                            command,
                            e.desc(),
                            e as i32
                        );
                        exit(1)
                    }
                }
            }
            Ok(ForkResult::Parent { child }) => {
                let stdin = stdin();
                let std_in_fd = stdin.as_fd();
                let _ = setpgid(child, child);
                let _ = tcsetpgrp(std_in_fd, child);
                match waitpid(child, None) {
                    Ok(_) => {}
                    Err(e) => eprintln!("Waitpid error: {}(Code: {})", e.desc(), e as i32),
                }
                let _ = tcsetpgrp(std_in_fd, getpgrp());
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
    /* ignoring some signals */
    let sa = SigAction::new(
        nix::sys::signal::SigHandler::SigIgn,
        SaFlags::empty(),
        SigSet::empty(),
    );
    unsafe {
        for signal in HANDLED_SIGNALS {
            let _ = sigaction(signal, &sa);
        }
    }
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
                        Commands::PWD => match env::current_dir() {
                            Ok(c) => println!("{}", c.display()),
                            Err(e) => eprintln!("Error with pwd {}", e),
                        },
                        Commands::CD => {
                            if args.len() > 0 {
                                handle_cd_command(args[0])
                            }
                        }
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
