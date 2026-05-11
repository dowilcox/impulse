use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct TextSpan {
    pub start: usize,
    pub end: usize,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellQuoteState {
    #[default]
    None,
    Single,
    Double,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellTokenRole {
    Assignment,
    Command,
    #[default]
    Argument,
    RedirectionOperator,
    RedirectionTarget,
    PipelineSeparator,
    ControlOperator,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShellToken {
    pub text: String,
    pub span: TextSpan,
    pub role: ShellTokenRole,
    pub quote_state: ShellQuoteState,
    pub terminated: bool,
    pub quoted: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ShellCompletionKind {
    Command,
    Argument,
    EnvAssignment,
    RedirectTarget,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShellCompletion {
    pub kind: ShellCompletionKind,
    pub prefix: String,
    pub span: TextSpan,
    pub quote_state: ShellQuoteState,
    pub command: Option<String>,
    pub command_span: Option<TextSpan>,
    pub argument_index: usize,
    pub previous_word: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShellRedirection {
    pub operator: String,
    pub operator_span: TextSpan,
    pub target: Option<String>,
    pub target_span: Option<TextSpan>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ShellParseResult {
    pub input: String,
    pub cursor: usize,
    pub tokens: Vec<ShellToken>,
    pub assignments: Vec<ShellToken>,
    pub redirects: Vec<ShellRedirection>,
    pub completion: ShellCompletion,
    pub pipeline_index: usize,
    pub incomplete: bool,
}

pub fn parse_shell_input(input: &str, cursor: usize) -> ShellParseResult {
    let cursor = clamp_cursor(input, cursor);
    let prefix = &input[..cursor];
    let (mut tokens, quote_state, escaped_at_end) = lex_prefix(prefix);
    classify_tokens(&mut tokens);

    let current_index = tokens
        .iter()
        .position(|token| token_contains_cursor(token, cursor));
    let segment_start = current_segment_start(&tokens);
    let pipeline_index = tokens[..segment_start]
        .iter()
        .filter(|token| token.role == ShellTokenRole::PipelineSeparator)
        .count();

    let assignments = tokens[segment_start..]
        .iter()
        .filter(|token| token.role == ShellTokenRole::Assignment)
        .cloned()
        .collect();
    let redirects = collect_redirects(&tokens[segment_start..]);
    let completion = build_completion(&tokens, current_index, segment_start, cursor, quote_state);

    ShellParseResult {
        input: input.to_string(),
        cursor,
        tokens,
        assignments,
        redirects,
        completion,
        pipeline_index,
        incomplete: escaped_at_end || quote_state != ShellQuoteState::None,
    }
}

fn clamp_cursor(input: &str, cursor: usize) -> usize {
    let mut cursor = cursor.min(input.len());
    while cursor > 0 && !input.is_char_boundary(cursor) {
        cursor -= 1;
    }
    cursor
}

fn lex_prefix(input: &str) -> (Vec<ShellToken>, ShellQuoteState, bool) {
    let mut tokens = Vec::new();
    let mut current: Option<ShellToken> = None;
    let mut quote_state = ShellQuoteState::None;
    let mut pos = 0;
    let mut escaped = false;
    let mut escaped_at_end = false;

    while pos < input.len() {
        let Some(ch) = input[pos..].chars().next() else {
            break;
        };
        let next_pos = pos + ch.len_utf8();

        if escaped {
            append_literal(&mut current, pos, next_pos, ch);
            escaped = false;
            pos = next_pos;
            continue;
        }

        match quote_state {
            ShellQuoteState::None => {
                if ch.is_whitespace() {
                    push_current(&mut tokens, &mut current, true, quote_state);
                    pos = next_pos;
                    continue;
                }

                if ch == '\\' {
                    if next_pos >= input.len() {
                        ensure_token(&mut current, pos).span.end = next_pos;
                        escaped_at_end = true;
                        pos = next_pos;
                    } else {
                        escaped = true;
                        ensure_token(&mut current, pos).span.end = next_pos;
                        pos = next_pos;
                    }
                    continue;
                }

                if ch == '\'' {
                    let token = ensure_token(&mut current, pos);
                    token.quoted = true;
                    token.span.end = next_pos;
                    quote_state = ShellQuoteState::Single;
                    pos = next_pos;
                    continue;
                }

                if ch == '"' {
                    let token = ensure_token(&mut current, pos);
                    token.quoted = true;
                    token.span.end = next_pos;
                    quote_state = ShellQuoteState::Double;
                    pos = next_pos;
                    continue;
                }

                if let Some((operator, end, role)) = control_operator_at(input, pos) {
                    push_current(&mut tokens, &mut current, true, quote_state);
                    tokens.push(operator_token(operator, pos, end, role));
                    pos = end;
                    continue;
                }

                if let Some((operator, end)) = redirect_operator_at(input, pos) {
                    if current
                        .as_ref()
                        .map(|token| !token.quoted && is_ascii_digits(&token.text))
                        .unwrap_or(false)
                    {
                        let mut token = current.take().expect("checked above");
                        token.text.push_str(&operator);
                        token.span.end = end;
                        token.role = ShellTokenRole::RedirectionOperator;
                        token.terminated = true;
                        tokens.push(token);
                    } else {
                        push_current(&mut tokens, &mut current, true, quote_state);
                        tokens.push(operator_token(
                            operator,
                            pos,
                            end,
                            ShellTokenRole::RedirectionOperator,
                        ));
                    }
                    pos = end;
                    continue;
                }

                append_literal(&mut current, pos, next_pos, ch);
            }
            ShellQuoteState::Single => {
                if ch == '\'' {
                    if let Some(token) = current.as_mut() {
                        token.span.end = next_pos;
                    }
                    quote_state = ShellQuoteState::None;
                } else {
                    append_literal(&mut current, pos, next_pos, ch);
                }
            }
            ShellQuoteState::Double => {
                if ch == '"' {
                    if let Some(token) = current.as_mut() {
                        token.span.end = next_pos;
                    }
                    quote_state = ShellQuoteState::None;
                } else if ch == '\\' {
                    if next_pos >= input.len() {
                        ensure_token(&mut current, pos).span.end = next_pos;
                        escaped_at_end = true;
                    } else {
                        escaped = true;
                        ensure_token(&mut current, pos).span.end = next_pos;
                    }
                } else {
                    append_literal(&mut current, pos, next_pos, ch);
                }
            }
        }

        pos = next_pos;
    }

    push_current(&mut tokens, &mut current, false, quote_state);
    (tokens, quote_state, escaped_at_end)
}

fn ensure_token(current: &mut Option<ShellToken>, start: usize) -> &mut ShellToken {
    current.get_or_insert_with(|| ShellToken {
        text: String::new(),
        span: TextSpan { start, end: start },
        role: ShellTokenRole::Argument,
        quote_state: ShellQuoteState::None,
        terminated: false,
        quoted: false,
    })
}

fn append_literal(current: &mut Option<ShellToken>, start: usize, end: usize, ch: char) {
    let token = ensure_token(current, start);
    token.text.push(ch);
    token.span.end = end;
}

fn push_current(
    tokens: &mut Vec<ShellToken>,
    current: &mut Option<ShellToken>,
    terminated: bool,
    quote_state: ShellQuoteState,
) {
    if let Some(mut token) = current.take() {
        token.terminated = terminated;
        token.quote_state = quote_state;
        tokens.push(token);
    }
}

fn operator_token(text: String, start: usize, end: usize, role: ShellTokenRole) -> ShellToken {
    ShellToken {
        text,
        span: TextSpan { start, end },
        role,
        quote_state: ShellQuoteState::None,
        terminated: true,
        quoted: false,
    }
}

fn control_operator_at(input: &str, pos: usize) -> Option<(String, usize, ShellTokenRole)> {
    let rest = &input[pos..];
    if rest.starts_with("||") || rest.starts_with("&&") {
        return Some((
            rest[..2].to_string(),
            pos + 2,
            ShellTokenRole::ControlOperator,
        ));
    }
    if rest.starts_with('|') {
        return Some(("|".to_string(), pos + 1, ShellTokenRole::PipelineSeparator));
    }
    if rest.starts_with(';') || rest.starts_with('&') {
        return Some((
            rest[..1].to_string(),
            pos + 1,
            ShellTokenRole::ControlOperator,
        ));
    }
    None
}

fn redirect_operator_at(input: &str, pos: usize) -> Option<(String, usize)> {
    let rest = &input[pos..];
    let mut bytes = rest.as_bytes();
    let mut consumed = 0;

    while let Some(byte) = bytes.first().copied() {
        if byte.is_ascii_digit() {
            consumed += 1;
            bytes = &rest.as_bytes()[consumed..];
        } else {
            break;
        }
    }

    let byte = bytes.first().copied()?;
    if byte != b'>' && byte != b'<' {
        return None;
    }
    consumed += 1;

    if let Some(next) = rest.as_bytes().get(consumed).copied() {
        if next == byte || next == b'&' {
            consumed += 1;
        }
    }

    Some((rest[..consumed].to_string(), pos + consumed))
}

fn classify_tokens(tokens: &mut [ShellToken]) {
    let mut command_seen = false;
    let mut expect_redirection_target = false;

    for token in tokens {
        match token.role {
            ShellTokenRole::PipelineSeparator | ShellTokenRole::ControlOperator => {
                command_seen = false;
                expect_redirection_target = false;
            }
            ShellTokenRole::RedirectionOperator => {
                expect_redirection_target = true;
            }
            _ if expect_redirection_target => {
                token.role = ShellTokenRole::RedirectionTarget;
                expect_redirection_target = false;
            }
            _ if !command_seen && is_assignment_word(&token.text) => {
                token.role = ShellTokenRole::Assignment;
            }
            _ if !command_seen => {
                token.role = ShellTokenRole::Command;
                command_seen = true;
            }
            _ => {
                token.role = ShellTokenRole::Argument;
            }
        }
    }
}

fn current_segment_start(tokens: &[ShellToken]) -> usize {
    tokens
        .iter()
        .rposition(|token| {
            matches!(
                token.role,
                ShellTokenRole::PipelineSeparator | ShellTokenRole::ControlOperator
            )
        })
        .map(|index| index + 1)
        .unwrap_or(0)
}

fn collect_redirects(tokens: &[ShellToken]) -> Vec<ShellRedirection> {
    let mut redirects = Vec::new();
    let mut iter = tokens.iter().peekable();
    while let Some(token) = iter.next() {
        if token.role != ShellTokenRole::RedirectionOperator {
            continue;
        }
        let target = iter
            .peek()
            .filter(|next| next.role == ShellTokenRole::RedirectionTarget);
        redirects.push(ShellRedirection {
            operator: token.text.clone(),
            operator_span: token.span,
            target: target.map(|target| target.text.clone()),
            target_span: target.map(|target| target.span),
        });
    }
    redirects
}

fn build_completion(
    tokens: &[ShellToken],
    current_index: Option<usize>,
    segment_start: usize,
    cursor: usize,
    quote_state: ShellQuoteState,
) -> ShellCompletion {
    let current = current_index.and_then(|index| tokens.get(index));
    let command = tokens[segment_start..]
        .iter()
        .find(|token| token.role == ShellTokenRole::Command);
    let previous_word = previous_word(tokens, current_index, segment_start);
    let expects_redirect_target = previous_role(tokens, current_index, segment_start)
        == Some(ShellTokenRole::RedirectionOperator);

    let kind = if current
        .map(|token| token.role == ShellTokenRole::RedirectionTarget)
        .unwrap_or(false)
        || expects_redirect_target
    {
        ShellCompletionKind::RedirectTarget
    } else if current
        .map(|token| token.role == ShellTokenRole::Assignment)
        .unwrap_or(false)
    {
        ShellCompletionKind::EnvAssignment
    } else if command.is_none()
        || current
            .map(|token| token.role == ShellTokenRole::Command)
            .unwrap_or(false)
    {
        ShellCompletionKind::Command
    } else {
        ShellCompletionKind::Argument
    };

    let prefix = current
        .filter(|token| !is_operator_role(token.role))
        .map(|token| token.text.clone())
        .unwrap_or_default();
    let span = current
        .filter(|token| !is_operator_role(token.role))
        .map(|token| token.span)
        .unwrap_or(TextSpan {
            start: cursor,
            end: cursor,
        });
    let argument_count_end = current_index.unwrap_or(tokens.len());
    let argument_index = tokens[segment_start..argument_count_end]
        .iter()
        .filter(|token| {
            matches!(
                token.role,
                ShellTokenRole::Argument | ShellTokenRole::RedirectionTarget
            )
        })
        .count();

    ShellCompletion {
        kind,
        prefix,
        span,
        quote_state: current
            .map(|token| token.quote_state)
            .unwrap_or(quote_state),
        command: command.map(|token| token.text.clone()),
        command_span: command.map(|token| token.span),
        argument_index,
        previous_word,
    }
}

fn token_contains_cursor(token: &ShellToken, cursor: usize) -> bool {
    !token.terminated
        && !is_operator_role(token.role)
        && token.span.start <= cursor
        && cursor <= token.span.end
}

fn previous_role(
    tokens: &[ShellToken],
    current_index: Option<usize>,
    segment_start: usize,
) -> Option<ShellTokenRole> {
    let end = current_index.unwrap_or(tokens.len());
    tokens[segment_start..end].last().map(|token| token.role)
}

fn previous_word(
    tokens: &[ShellToken],
    current_index: Option<usize>,
    segment_start: usize,
) -> Option<String> {
    let end = current_index.unwrap_or(tokens.len());
    tokens[segment_start..end]
        .iter()
        .rev()
        .find(|token| !is_operator_role(token.role))
        .map(|token| token.text.clone())
}

fn is_operator_role(role: ShellTokenRole) -> bool {
    matches!(
        role,
        ShellTokenRole::RedirectionOperator
            | ShellTokenRole::PipelineSeparator
            | ShellTokenRole::ControlOperator
    )
}

fn is_assignment_word(word: &str) -> bool {
    let Some(eq) = word.find('=') else {
        return false;
    };
    let name = &word[..eq];
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_ascii_digits(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_env_assignment_and_command_context() {
        let input = "FOO=bar cargo test -p impulse-core";
        let parsed = parse_shell_input(input, input.len());

        assert_eq!(parsed.assignments.len(), 1);
        assert_eq!(parsed.assignments[0].text, "FOO=bar");
        assert_eq!(parsed.completion.command.as_deref(), Some("cargo"));
        assert_eq!(parsed.completion.kind, ShellCompletionKind::Argument);
        assert_eq!(parsed.completion.prefix, "impulse-core");
    }

    #[test]
    fn preserves_quoted_argument_and_reports_unfinished_quote() {
        let input = "cd \"src/my folder";
        let parsed = parse_shell_input(input, input.len());

        assert!(parsed.incomplete);
        assert_eq!(parsed.completion.command.as_deref(), Some("cd"));
        assert_eq!(parsed.completion.kind, ShellCompletionKind::Argument);
        assert_eq!(parsed.completion.prefix, "src/my folder");
        assert_eq!(parsed.completion.quote_state, ShellQuoteState::Double);
    }

    #[test]
    fn handles_escaped_spaces() {
        let input = "echo hello\\ world";
        let parsed = parse_shell_input(input, input.len());

        assert!(!parsed.incomplete);
        assert_eq!(parsed.completion.command.as_deref(), Some("echo"));
        assert_eq!(parsed.completion.prefix, "hello world");
        assert_eq!(parsed.tokens[1].text, "hello world");
    }

    #[test]
    fn resets_command_context_after_pipe() {
        let input = "cat Cargo.toml | rg serde";
        let parsed = parse_shell_input(input, input.len());

        assert_eq!(parsed.pipeline_index, 1);
        assert_eq!(parsed.completion.command.as_deref(), Some("rg"));
        assert_eq!(parsed.completion.kind, ShellCompletionKind::Argument);
        assert_eq!(parsed.completion.prefix, "serde");
    }

    #[test]
    fn parses_redirect_operator_and_target() {
        let input = "cargo test > target/out.log";
        let parsed = parse_shell_input(input, input.len());

        assert_eq!(parsed.redirects.len(), 1);
        assert_eq!(parsed.redirects[0].operator, ">");
        assert_eq!(
            parsed.redirects[0].target.as_deref(),
            Some("target/out.log")
        );
        assert_eq!(parsed.completion.kind, ShellCompletionKind::RedirectTarget);
        assert_eq!(parsed.completion.prefix, "target/out.log");
    }

    #[test]
    fn parses_fd_redirect_without_spaces() {
        let input = "cargo test 2>/tmp/impulse.log";
        let parsed = parse_shell_input(input, input.len());

        assert_eq!(parsed.redirects.len(), 1);
        assert_eq!(parsed.redirects[0].operator, "2>");
        assert_eq!(
            parsed.redirects[0].target.as_deref(),
            Some("/tmp/impulse.log")
        );
        assert_eq!(parsed.completion.kind, ShellCompletionKind::RedirectTarget);
    }

    #[test]
    fn expects_redirect_target_after_operator() {
        let input = "cargo test 2>";
        let parsed = parse_shell_input(input, input.len());

        assert_eq!(parsed.redirects.len(), 1);
        assert_eq!(parsed.redirects[0].target, None);
        assert_eq!(parsed.completion.kind, ShellCompletionKind::RedirectTarget);
        assert_eq!(parsed.completion.span, TextSpan { start: 13, end: 13 });
    }

    #[test]
    fn reports_trailing_escape_as_incomplete() {
        let input = "echo foo\\";
        let parsed = parse_shell_input(input, input.len());

        assert!(parsed.incomplete);
        assert_eq!(parsed.completion.prefix, "foo");
        assert_eq!(parsed.completion.kind, ShellCompletionKind::Argument);
    }
}
