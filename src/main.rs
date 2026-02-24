mod lexer;
use nix::{
    errno::Errno,
    fcntl::OFlag,
    sys::{
        signal::{
            SaFlags, SigAction, SigSet,
            Signal::{self, SIGINT, SIGQUIT, SIGTSTP, SIGTTIN, SIGTTOU},
            sigaction,
        },
        stat::Mode,
        wait::waitpid,
    },
    unistd::{
        ForkResult, Pid, dup2_stderr, dup2_stdout, execv, fork, getpgrp, getpid, setpgid, tcsetpgrp,
    },
};
use std::{
    env::{self},
    ffi::CString,
    io::{BufRead, Write, stderr, stdout},
    os::{
        fd::{AsFd, AsRawFd, OwnedFd},
        unix::{ffi::OsStrExt, fs::PermissionsExt},
    },
    path::{Path, PathBuf},
    process::exit,
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
    redirects: Vec<lexer::Redirect>,
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
                            eprintln!("Pid: {}, invalid argument (contains NULL)", pid);
                            exit(1);
                        }
                    };
                    args_for_new_proc.push(other_argv);
                }
                if let Ok(_kept_alive_fds) = setup_redirects(redirects) {
                } else {
                    exit(1);
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

fn setup_redirects(redirects: Vec<lexer::Redirect>) -> Result<Vec<OwnedFd>, ()> {
    let default_flags = OFlag::O_WRONLY | OFlag::O_CREAT;
    let file_mode = Mode::S_IRUSR | Mode::S_IWUSR | Mode::S_IRGRP | Mode::S_IROTH;
    let mut keep_alive_fds = vec![];
    for redirect in redirects {
        let flags = default_flags
            | (if redirect.append {
                OFlag::O_APPEND
            } else {
                OFlag::O_TRUNC
            });
        let new_fd = match nix::fcntl::open(redirect.path.as_str(), flags, file_mode) {
            Ok(fd) => fd,
            Err(e) => {
                eprintln!("Couldnt open file: {}(Code: {})", e.desc(), e as i32);
                return Err(());
            }
        };
        let dup2_error_closure = |e: Errno| {
            eprintln!("Couldnt dup2: {}(Code: {})", e.desc(), e as i32);
            return Err(());
        };
        match redirect.fd {
            0 => return Err(()),
            1 => match dup2_stdout(new_fd) {
                Ok(_) => {}
                Err(e) => return dup2_error_closure(e),
            },
            2 => match dup2_stderr(new_fd) {
                Ok(_) => {}
                Err(e) => return dup2_error_closure(e),
            },
            _ => match unsafe { nix::unistd::dup2_raw(new_fd, redirect.fd as i32) } {
                Ok(new_redirect_fd) => {
                    keep_alive_fds.push(new_redirect_fd);
                }
                Err(e) => return dup2_error_closure(e),
            },
        }
    }
    return Ok(keep_alive_fds);
}

fn restore_fds(stdout_copy: Option<OwnedFd>, stderr_copy: Option<OwnedFd>) -> Result<(), ()> {
    if let Some(stdout_fd) = stdout_copy {
        if let Err(_) = dup2_stdout(stdout_fd) {
            return Err(());
        }
    }
    if let Some(stderr_fd) = stderr_copy {
        if let Err(_) = dup2_stderr(stderr_fd) {
            return Err(());
        }
    }
    Ok(())
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
        if let Err(e) = stdout().flush() {
            eprintln!("Error flushing stdout: {}", e);
            return;
        }
        match stdin_handle.read_line(&mut stdin_buffer) {
            Ok(0) => break,
            Ok(_) => {
                let parsed_command = lexer::tokenize(&stdin_buffer);
                if !parsed_command.command.is_empty() {
                    let command = parsed_command.command.as_str();
                    let args = &parsed_command.args;
                    if is_builtin(&command) {
                        let mut stdout_copy = None;
                        let mut stderr_copy = None;
                        let did_redirect_stdout = parsed_command
                            .redirects
                            .iter()
                            .any(|r| r.fd == stdout().as_raw_fd() as u32);
                        let did_redirect_stderr = parsed_command
                            .redirects
                            .iter()
                            .any(|r| r.fd == stderr().as_raw_fd() as u32);
                        if did_redirect_stdout {
                            stdout_copy = match nix::unistd::dup(stdout().as_fd()) {
                                Ok(stdout_copy) => Some(stdout_copy),
                                Err(_) => {
                                    eprintln!("Couldnt dup stdout");
                                    continue;
                                }
                            }
                        }
                        if did_redirect_stderr {
                            stderr_copy = match nix::unistd::dup(stderr().as_fd()) {
                                Ok(stderr_copy) => Some(stderr_copy),
                                Err(_) => {
                                    eprintln!("Couldnt dup stderr");
                                    continue;
                                }
                            }
                        }
                        if let Err(_) = setup_redirects(parsed_command.redirects) {
                            if let Err(_) = restore_fds(stdout_copy, stderr_copy) {
                                eprintln!("Couldnt restore fds");
                                break;
                            }
                            continue;
                        }
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
                            _ => unreachable!(),
                        }
                        if let Err(_) = restore_fds(stdout_copy, stderr_copy) {
                            eprintln!("Couldnt restore fds");
                            break;
                        }
                    } else {
                        handle_non_builtins(
                            shell_pgid,
                            stdin_handle.as_fd(),
                            command,
                            &args,
                            parsed_command.redirects,
                        );
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
