//! Subconjunto léxico deliberadamente limitado de POSIX shell.
const MAX_BYTES: usize = 16_384;
const MAX_TOKENS: usize = 1024;
type Result<T> = std::result::Result<T, &'static str>;

#[derive(Debug, PartialEq, Eq)]
enum Token {
    Word(String),
    Separator(bool),
    Semicolon,
    Redirect,
}

fn lex(input: &str) -> Result<Vec<Token>> {
    if input.is_empty() || input.len() > MAX_BYTES {
        return Err("guardrails: invalid shell length");
    }
    let mut chars = input.chars().peekable();
    let mut tokens = Vec::new();
    let mut word = String::new();
    let mut started = false;
    let mut quote = None;
    while let Some(ch) = chars.next() {
        if ch.is_control() && !matches!(ch, '\n' | '\t') {
            return Err("guardrails: shell control characters are unsupported");
        }
        if let Some(delimiter) = quote {
            if ch == delimiter {
                quote = None;
            } else if delimiter == '"' && matches!(ch, '$' | '`') {
                return Err("guardrails: shell expansion is unsupported");
            } else if delimiter == '"' && ch == '\\' {
                let next = chars.next().ok_or("guardrails: incomplete shell escape")?;
                if !matches!(next, '$' | '`' | '"' | '\\') {
                    return Err("guardrails: unsupported quoted escape");
                }
                word.push(next);
            } else {
                word.push(ch);
            }
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                started = true;
            }
            '\\' => {
                let next = chars.next().ok_or("guardrails: incomplete shell escape")?;
                if next.is_control() {
                    return Err("guardrails: unsupported shell continuation");
                }
                word.push(next);
                started = true;
            }
            '$' | '`' | '*' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '~' => {
                return Err("guardrails: shell expansion or compound syntax is unsupported");
            }
            '#' if !started => {
                for next in chars.by_ref() {
                    if next == '\n' {
                        tokens.push(Token::Separator(false));
                        break;
                    }
                }
            }
            ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' => {
                // Reject attached numeric descriptors rather than treating them as argv.
                // This also conservatively rejects quoted/escaped numeric words.
                if matches!(ch, '<' | '>')
                    && started
                    && !word.is_empty()
                    && word.bytes().all(|byte| byte.is_ascii_digit())
                {
                    return Err("guardrails: attached numeric shell redirection is unsupported");
                }
                if started {
                    tokens.push(Token::Word(std::mem::take(&mut word)));
                    started = false;
                }
                match ch {
                    '\n' => tokens.push(Token::Separator(false)),
                    ';' => tokens.push(Token::Semicolon),
                    '&' if chars.next_if_eq(&'&').is_some() => {
                        tokens.push(Token::Separator(true));
                        if chars.peek().is_none() {
                            return Err("guardrails: incomplete shell operator");
                        }
                    }
                    '&' => return Err("guardrails: background shell is unsupported"),
                    '|' => {
                        chars.next_if_eq(&'|');
                        tokens.push(Token::Separator(true));
                        if chars.peek().is_none() {
                            return Err("guardrails: incomplete shell operator");
                        }
                    }
                    '<' | '>' => {
                        if ch == '>' {
                            chars.next_if_eq(&'>');
                        }
                        if chars
                            .peek()
                            .is_some_and(|c| matches!(c, '<' | '>' | '&' | '|'))
                        {
                            return Err("guardrails: unsupported shell redirection");
                        }
                        tokens.push(Token::Redirect);
                    }
                    _ => {}
                }
            }
            _ => {
                word.push(ch);
                started = true;
            }
        }
        if tokens.len() > MAX_TOKENS {
            return Err("guardrails: shell token limit exceeded");
        }
    }
    if quote.is_some() {
        return Err("guardrails: unterminated shell quote");
    }
    if started {
        tokens.push(Token::Word(word));
    }
    if tokens.len() > MAX_TOKENS {
        return Err("guardrails: shell token limit exceeded");
    }
    Ok(tokens)
}

#[derive(Default)]
pub struct Command {
    pub args: Vec<String>,
    pub redirects: Vec<String>,
}

pub fn words(input: &str) -> Result<Vec<Command>> {
    let tokens = lex(input)?;
    let mut commands = Vec::new();
    let mut command = Command::default();
    let mut redirect = false;
    let mut needs_command = false;
    for token in tokens {
        match token {
            Token::Word(word) => {
                if word.is_empty() && command.args.is_empty() {
                    return Err("guardrails: empty shell executable");
                }
                if redirect {
                    command.redirects.push(word);
                } else {
                    command.args.push(word);
                }
                redirect = false;
                needs_command = false;
            }
            Token::Redirect => {
                if command.args.is_empty() || redirect {
                    return Err("guardrails: unsupported shell redirection position");
                }
                redirect = true;
            }
            Token::Semicolon => {
                if redirect || needs_command || command.args.is_empty() {
                    return Err("guardrails: invalid shell operator sequence");
                }
                commands.push(std::mem::take(&mut command));
            }
            Token::Separator(required) => {
                if redirect || (needs_command && required) || (required && command.args.is_empty())
                {
                    return Err("guardrails: invalid shell operator sequence");
                }
                if !command.args.is_empty() {
                    commands.push(std::mem::take(&mut command));
                }
                needs_command |= required;
            }
        }
    }
    if redirect || needs_command {
        return Err("guardrails: missing shell redirection destination");
    }
    if !command.args.is_empty() {
        commands.push(command);
    }
    if commands.is_empty() {
        return Err("guardrails: missing shell command");
    }
    Ok(commands)
}

pub fn check(input: &str, blocked: &[String]) -> Result<()> {
    for command in words(input)? {
        let command = command.args;
        if command[0].contains('=') {
            return Err("guardrails: indirect or compound shell execution is unsupported");
        }
        let executable = command[0]
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();
        let executable = executable.as_str();
        if blocked
            .iter()
            .any(|name| name.eq_ignore_ascii_case(executable))
        {
            return Err("guardrails: project blocks this executable");
        }
        if matches!(
            executable,
            "if" | "then"
                | "else"
                | "elif"
                | "fi"
                | "for"
                | "while"
                | "until"
                | "do"
                | "done"
                | "case"
                | "esac"
                | "in"
                | "!"
                | "sh"
                | "bash"
                | "dash"
                | "zsh"
                | "ksh"
                | "fish"
                | "eval"
                | "alias"
                | "trap"
                | "source"
                | "."
                | "exec"
                | "command"
                | "builtin"
                | "env"
                | "sudo"
                | "doas"
                | "xargs"
                | "nohup"
                | "timeout"
                | "time"
                | "nice"
                | "watch"
                | "parallel"
                | "cd"
                | "chdir"
                | "pushd"
                | "popd"
        ) {
            return Err("guardrails: indirect or compound shell execution is unsupported");
        }
        if matches!(
            executable,
            "dd" | "shred" | "wipe" | "mkfs" | "fdisk" | "diskutil"
        ) || executable.starts_with("mkfs.")
        {
            return Err("guardrails: destructive executable blocked");
        }
        if executable == "rm"
            && command
                .iter()
                .skip(1)
                .take_while(|arg| arg.as_str() != "--")
                .any(|arg| {
                    arg.starts_with("--")
                        || (arg.starts_with('-')
                            && !arg.starts_with("--")
                            && arg.contains(['r', 'R']))
                })
        {
            return Err("guardrails: recursive removal or unsupported rm long option blocked");
        }
        if executable == "git" {
            let Some(subcommand) = command.get(1) else {
                continue;
            };
            if subcommand.starts_with('-') {
                return Err("guardrails: git global options are unsupported");
            }
            if subcommand == "reset"
                && command
                    .iter()
                    .skip(2)
                    .take_while(|arg| arg.as_str() != "--")
                    .any(|arg| arg.starts_with("--h"))
            {
                return Err("guardrails: git reset --hard blocked");
            }
            if subcommand == "clean" {
                let mut dry_run = false;
                for arg in command
                    .iter()
                    .skip(2)
                    .take_while(|arg| arg.as_str() != "--")
                {
                    match arg.as_str() {
                        "--dry-run" => dry_run = true,
                        "--force" | "--quiet" => {}
                        option if option.starts_with('-') => {
                            if option.starts_with("--")
                                || option.len() == 1
                                || !option[1..].chars().all(|ch| "dfnqxX".contains(ch))
                            {
                                return Err("guardrails: unsupported git clean option");
                            }
                            dry_run |= option.contains('n');
                        }
                        _ => {}
                    }
                }
                if !dry_run {
                    return Err("guardrails: destructive git clean blocked");
                }
            }
        }
        if executable == "find"
            && command.iter().any(|arg| {
                matches!(
                    arg.as_str(),
                    "-exec" | "-execdir" | "-ok" | "-okdir" | "-delete"
                )
            })
        {
            return Err("guardrails: find execution or deletion blocked");
        }
    }
    Ok(())
}
