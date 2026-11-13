//! Deliberately limited lexical subset of POSIX shell.
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
        if quote.is_some() {
            push_quoted(&mut chars, &mut quote, &mut word, ch)?;
            continue;
        }
        match ch {
            '\'' | '"' => {
                quote = Some(ch);
                started = true;
            }
            '\\' => {
                push_escape(&mut chars, &mut word)?;
                started = true;
            }
            '$' | '`' | '*' | '?' | '[' | ']' | '(' | ')' | '{' | '}' | '~' => {
                return Err("guardrails: shell expansion or compound syntax is unsupported");
            }
            '#' if !started => skip_comment(&mut chars, &mut tokens),
            ' ' | '\t' | '\n' | ';' | '&' | '|' | '<' | '>' => {
                push_delimiter(ch, &mut chars, &mut tokens, &mut word, &mut started)?;
            }
            _ => {
                word.push(ch);
                started = true;
            }
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

/// Consumes one character inside a quoted span, opening and closing the quote.
fn push_quoted(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    quote: &mut Option<char>,
    word: &mut String,
    ch: char,
) -> Result<()> {
    let Some(delimiter) = *quote else {
        return Ok(());
    };
    if ch == delimiter {
        *quote = None;
        return Ok(());
    }
    if delimiter != '"' {
        word.push(ch);
        return Ok(());
    }
    if matches!(ch, '$' | '`') {
        return Err("guardrails: shell expansion is unsupported");
    }
    if ch != '\\' {
        word.push(ch);
        return Ok(());
    }
    let next = chars.next().ok_or("guardrails: incomplete shell escape")?;
    if !matches!(next, '$' | '`' | '"' | '\\') {
        return Err("guardrails: unsupported quoted escape");
    }
    word.push(next);
    Ok(())
}

/// Consumes an unquoted backslash escape.
fn push_escape(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    word: &mut String,
) -> Result<()> {
    let next = chars.next().ok_or("guardrails: incomplete shell escape")?;
    if next.is_control() {
        return Err("guardrails: unsupported shell continuation");
    }
    word.push(next);
    Ok(())
}

/// Discards a comment up to and including its newline, which ends the command line.
fn skip_comment(chars: &mut std::iter::Peekable<std::str::Chars<'_>>, tokens: &mut Vec<Token>) {
    for next in chars.by_ref() {
        if next == '\n' {
            tokens.push(Token::Separator(false));
            break;
        }
    }
}

/// Closes the pending word and records the separator, operator or redirect it precedes.
fn push_delimiter(
    ch: char,
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
    tokens: &mut Vec<Token>,
    word: &mut String,
    started: &mut bool,
) -> Result<()> {
    // Reject attached numeric descriptors rather than treating them as argv.
    // This also conservatively rejects quoted/escaped numeric words.
    if matches!(ch, '<' | '>')
        && *started
        && !word.is_empty()
        && word.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err("guardrails: attached numeric shell redirection is unsupported");
    }
    if *started {
        tokens.push(Token::Word(std::mem::take(word)));
        *started = false;
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
    if tokens.len() > MAX_TOKENS {
        return Err("guardrails: shell token limit exceeded");
    }
    Ok(())
}

#[derive(Default)]
pub struct Command {
    pub args: Vec<String>,
    pub redirects: Vec<String>,
}

/// Executables that would run the command through another interpreter or shell.
const INDIRECT_EXECUTABLES: &[&str] = &[
    "if", "then", "else", "elif", "fi", "for", "while", "until", "do", "done", "case", "esac",
    "in", "!", "sh", "bash", "dash", "zsh", "ksh", "fish", "eval", "alias", "trap", "source", ".",
    "exec", "command", "builtin", "env", "sudo", "doas", "xargs", "nohup", "timeout", "time",
    "nice", "watch", "parallel", "cd", "chdir", "pushd", "popd",
];

/// Executables whose normal use destroys data irrecoverably.
const DESTRUCTIVE_EXECUTABLES: &[&str] = &["dd", "shred", "wipe", "mkfs", "fdisk", "diskutil"];

/// Options that make `find` run or delete whatever it is given.
const FIND_EXECUTION: &[&str] = &["-exec", "-execdir", "-ok", "-okdir", "-delete"];

pub fn words(input: &str) -> Result<Vec<Command>> {
    let tokens = lex(input)?;
    let mut commands = Vec::new();
    let mut command = Command::default();
    let mut redirect = false;
    let mut needs_command = false;
    for token in tokens {
        match token {
            Token::Word(word) => {
                push_word(&mut command, word, &mut redirect, &mut needs_command)?;
            }
            Token::Redirect => {
                start_redirect(&command, redirect)?;
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

fn push_word(
    command: &mut Command,
    word: String,
    redirect: &mut bool,
    needs_command: &mut bool,
) -> Result<()> {
    if word.is_empty() && command.args.is_empty() {
        return Err("guardrails: empty shell executable");
    }
    if *redirect {
        command.redirects.push(word);
    } else {
        command.args.push(word);
    }
    *redirect = false;
    *needs_command = false;
    Ok(())
}

const fn start_redirect(command: &Command, redirect: bool) -> Result<()> {
    if command.args.is_empty() || redirect {
        return Err("guardrails: unsupported shell redirection position");
    }
    Ok(())
}

pub fn check(input: &str, blocked: &[String]) -> Result<()> {
    for command in words(input)? {
        reject_command(&command.args, blocked)?;
    }
    Ok(())
}

/// Applies every blocking rule to one tokenized command.
fn reject_command(command: &[String], blocked: &[String]) -> Result<()> {
    if command[0].contains('=') {
        return Err("guardrails: indirect or compound shell execution is unsupported");
    }
    let executable = command[0]
        .rsplit('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    if blocked
        .iter()
        .any(|name| name.eq_ignore_ascii_case(&executable))
    {
        return Err("guardrails: project blocks this executable");
    }
    if INDIRECT_EXECUTABLES.contains(&executable.as_str()) {
        return Err("guardrails: indirect or compound shell execution is unsupported");
    }
    if DESTRUCTIVE_EXECUTABLES.contains(&executable.as_str()) || executable.starts_with("mkfs.") {
        return Err("guardrails: destructive executable blocked");
    }
    if executable == "rm" {
        reject_recursive_removal(command)?;
    }
    if executable == "git" {
        reject_destructive_git(command)?;
    }
    if executable == "find"
        && command
            .iter()
            .any(|arg| FIND_EXECUTION.contains(&arg.as_str()))
    {
        return Err("guardrails: find execution or deletion blocked");
    }
    Ok(())
}

/// Rejects removal invocations that would recurse or use an unsupported long option.
fn reject_recursive_removal(command: &[String]) -> Result<()> {
    let recurses = command
        .iter()
        .skip(1)
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| {
            arg.starts_with("--")
                || (arg.starts_with('-') && !arg.starts_with("--") && arg.contains(['r', 'R']))
        });
    if recurses {
        return Err("guardrails: recursive removal or unsupported rm long option blocked");
    }
    Ok(())
}

/// Rejects the `git` subcommands that rewrite or discard work.
fn reject_destructive_git(command: &[String]) -> Result<()> {
    let Some(subcommand) = command.get(1) else {
        return Ok(());
    };
    if subcommand.starts_with('-') {
        return Err("guardrails: git global options are unsupported");
    }
    if subcommand == "reset" {
        let resets_hard = command
            .iter()
            .skip(2)
            .take_while(|arg| arg.as_str() != "--")
            .any(|arg| arg.starts_with("--h"));
        if resets_hard {
            return Err("guardrails: git reset --hard blocked");
        }
    }
    if subcommand == "clean" {
        reject_git_clean_without_dry_run(command)?;
    }
    Ok(())
}

/// `git clean` is only allowed when every invocation is a dry run.
fn reject_git_clean_without_dry_run(command: &[String]) -> Result<()> {
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
    Ok(())
}
