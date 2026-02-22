#[derive(Debug)]
pub struct Redirect {
    pub fd: u32,
    pub path: String,
    pub append: bool,
}

#[derive(Debug)]
pub struct ParsedCommand {
    pub command: String,
    pub args: Vec<String>,
    pub redirects: Vec<Redirect>,
}

pub fn tokenize(input: &str) -> ParsedCommand {
    let mut parsed_command = ParsedCommand {
        command: "".to_string(),
        args: vec![],
        redirects: vec![],
    };
    let mut current_token = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut is_escaped = false;
    let mut pending_redirect: Option<(u32, bool)> = None;
    let mut characters = input.chars().peekable();

    // println!("Chars: {:?}", characters);

    while let Some(c) = characters.next() {
        match c {
            '>' if !(in_single_quote || in_double_quote || is_escaped) => {
                let fd = if let Ok(fd) = current_token.parse::<u32>() {
                    current_token.clear();
                    fd
                } else {
                    if !current_token.is_empty() {
                        if parsed_command.command.is_empty() {
                            parsed_command.command = current_token.clone();
                        } else {
                            parsed_command.args.push(current_token.clone());
                        }
                        current_token.clear();
                    }
                    1
                };
                let append = if characters.peek() == Some(&'>') {
                    characters.next();
                    true
                } else {
                    false
                };
                pending_redirect = Some((fd, append));
            }
            '\\' if !(in_single_quote
                || is_escaped
                || (in_double_quote
                    && ![
                        Some(&'\"'),
                        Some(&'\\'),
                        Some(&'$'),
                        Some(&'`'),
                        Some(&'\n'),
                    ]
                    .contains(&characters.peek()))) =>
            {
                is_escaped = true
            }
            '\n' => {}
            '\'' if !(is_escaped || in_double_quote) => {
                in_single_quote = !in_single_quote;
            }
            '\"' if !(is_escaped || in_single_quote) => {
                in_double_quote = !in_double_quote;
            }
            ' ' | '\t' if !(is_escaped || in_single_quote || in_double_quote) => {
                if !current_token.is_empty() {
                    build_parsed_command(
                        &mut parsed_command,
                        &mut current_token,
                        &mut pending_redirect,
                    );
                }
            }
            _ => {
                current_token.push(c);
                is_escaped = false;
            }
        }
    }

    if !current_token.is_empty() {
        build_parsed_command(
            &mut parsed_command,
            &mut current_token,
            &mut pending_redirect,
        );
    }
    // println!("Tokens: {:?}", parsed_command);
    parsed_command
}

fn build_parsed_command(
    parsed_command: &mut ParsedCommand,
    current_token: &mut String,
    pending_redirect: &mut Option<(u32, bool)>,
) {
    if let Some((fd, append)) = pending_redirect.take() {
        parsed_command.redirects.push(Redirect {
            fd,
            path: current_token.clone(),
            append,
        });
    } else if parsed_command.command.is_empty() {
        parsed_command.command = current_token.clone();
    } else {
        parsed_command.args.push(current_token.clone());
    }
    current_token.clear();
}
