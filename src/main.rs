use std::{
    env::{self},
    ffi::CString,
    io::{BufRead, Write, stdout},
    os::{
        fd::AsFd,
        unix::{ffi::OsStrExt, fs::PermissionsExt},
    },
    path::{Path, PathBuf},
    process::exit,
};

use nix::{
    errno::Errno,
    sys::{
        signal::{
            SaFlags, SigAction, SigSet,
            Signal::{self, SIGINT, SIGQUIT, SIGTSTP, SIGTTIN, SIGTTOU},
            sigaction,
        },
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

const BUILTINS: [&str; 5] = [
    Commands::ECHO,
    Commands::EXIT,
    Commands::TYPE,
    Commands::PWD,
    Commands::CD,
];

const HANDLED_SIGNALS: [Signal; 5] = [SIGTTOU, SIGTTIN, SIGINT, SIGQUIT, SIGTSTP];

fn handle_type_command(args: &[String]) {
    for arg in args {
        if is_builtin(arg) {
            println!("{} is a shell builtin", arg);
        } else if let Some(full_path) = is_command_in_paths_env(arg.as_str()) {
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
    let old_pwd = env::current_dir().ok();
    let external_path;
    let path = if path == "-" {
        external_path = env::var_os("OLDPWD")
            .map(|home| PathBuf::from(home))
            .unwrap_or_default();
        external_path.as_path()
    } else if path == "~" {
        external_path = env::var_os("HOME")
            .map(|home| PathBuf::from(home))
            .unwrap_or_default();
        external_path.as_path()
    } else {
        Path::new(path)
    };
    if let Err(e) = env::set_current_dir(&path) {
        match e.kind() {
            std::io::ErrorKind::NotFound => {
                println!("cd: {}: No such file or directory", path.display())
            }
            std::io::ErrorKind::PermissionDenied => {
                println!("cd: {}: Permission denied", path.display());
            }
            _ => println!("cd: {}: {}", path.display(), e),
        }
    } else {
        if let Some(old) = old_pwd {
            unsafe {
                env::set_var("OLDPWD", old);
            }
        }
        if let Ok(new) = env::current_dir() {
            unsafe {
                env::set_var("PWD", new);
            }
        }
    }
}

fn handle_non_builtins(
    shell_pgid: Pid,
    tty: std::os::fd::BorrowedFd<'_>,
    command: &str,
    args: &[String],
) {
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
                match setpgid(Pid::from_raw(0), Pid::from_raw(0)) {
                    Ok(_) | Err(nix::errno::Errno::EACCES) | Err(nix::errno::Errno::EINVAL) => {}
                    Err(e) => {
                        eprintln!(
                            "Pid: {}, setpgid failed: {}(Code: {})",
                            pid,
                            e.desc(),
                            e as i32
                        );
                        exit(1);
                    }
                }
                let full_path = match CString::new(full_path.as_os_str().as_bytes()) {
                    Ok(p) => p,
                    Err(_) => {
                        eprintln!("Pid: {}, invalid executable path (contains NUL)", pid);
                        exit(1);
                    }
                };
                let mut args_for_new_proc = Vec::with_capacity(args.len() + 1);
                let argv0 = match CString::new(command.as_bytes()) {
                    Ok(c) => c,
                    Err(_) => {
                        eprintln!("Pid: {}, invalid argv[0]", pid);
                        exit(1);
                    }
                };
                args_for_new_proc.push(argv0);
                for arg in args {
                    let other_argv = match CString::new(arg.as_bytes()) {
                        Ok(c) => c,
                        Err(_) => {
                            eprintln!("Pid: {}, invalid argument (contains NUL)", pid);
                            exit(1);
                        }
                    };
                    args_for_new_proc.push(other_argv);
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
                match setpgid(child, child) {
                    Ok(_) | Err(nix::errno::Errno::EACCES) | Err(nix::errno::Errno::ESRCH) => {}
                    Err(e) => {
                        eprintln!("Parent setpgid failed: {}(Code: {})", e.desc(), e as i32);
                    }
                }
                if let Err(e) = tcsetpgrp(tty, child) {
                    eprintln!(
                        "Setting child as foreground process group failed: {} (Code: {})",
                        e.desc(),
                        e as i32
                    );
                }
                loop {
                    match waitpid(child, None) {
                        Ok(_) => break,
                        Err(Errno::EINTR) => {}
                        Err(e) => {
                            eprintln!("Waitpid error: {}(Code: {})", e.desc(), e as i32);
                            break;
                        }
                    }
                }
                if let Err(e) = tcsetpgrp(tty, shell_pgid) {
                    eprintln!(
                        "Setting parent as foreground process group failed: {} (Code: {})",
                        e.desc(),
                        e as i32
                    );
                }
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
    let shell_pgid = getpgrp();
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
                let tokens = tokenize(&stdin_buffer);
                if !tokens.is_empty() {
                    let command = tokens[0].as_str();
                    let args = &tokens[1..];
                    match command {
                        Commands::EXIT => break,
                        Commands::ECHO => println!("{}", args.join(" ")),
                        Commands::TYPE => handle_type_command(&args),
                        Commands::PWD => match env::current_dir() {
                            Ok(c) => println!("{}", c.display()),
                            Err(e) => eprintln!("Error with pwd {}", e),
                        },
                        Commands::CD => {
                            handle_cd_command(args.first().map(|s| s.as_str()).unwrap_or(&"~"))
                        }
                        _ => handle_non_builtins(shell_pgid, stdin_handle.as_fd(), command, &args),
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

fn tokenize(input: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current_token = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut is_escaped = false;
    let mut characters = input.chars().peekable();

    // println!("Chars: {:?}", characters);

    while let Some(c) = characters.next() {
        match c {
            '\\' if !is_escaped => is_escaped = true,
            '\n' => {}
            '\'' if !(is_escaped || in_double_quote) => {
                in_single_quote = !in_single_quote;
            }
            '\"' if !is_escaped => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !(is_escaped || in_single_quote || in_double_quote) => {
                if !current_token.is_empty() {
                    tokens.push(current_token);
                    current_token = String::new();
                }
            }
            _ => {
                current_token.push(c);
                is_escaped = false;
            }
        }
    }

    if !current_token.is_empty() {
        tokens.push(current_token);
    }
    // println!("Tokens: {:?}", tokens);
    tokens
}
