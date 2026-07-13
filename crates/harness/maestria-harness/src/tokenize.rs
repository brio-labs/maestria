use maestria_ports::PortError;

pub(crate) fn tokenize(raw: &str) -> Result<Vec<String>, PortError> {
    let chars: Vec<char> = raw.chars().collect();
    let mut tokens: Vec<String> = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        if chars[i].is_ascii_whitespace() {
            i += 1;
            continue;
        }
        let mut token = String::new();
        if chars[i] == '\'' {
            i += 1;
            while i < chars.len() && chars[i] != '\'' {
                token.push(chars[i]);
                i += 1;
            }
            if i >= chars.len() {
                return Err(PortError::InvalidInput {
                    message: "unterminated single quote".to_string(),
                });
            }
            i += 1;
        } else if chars[i] == '"' {
            i += 1;
            while i < chars.len() && chars[i] != '"' {
                if chars[i] == '\\' && i + 1 < chars.len() {
                    i += 1;
                    match chars[i] {
                        '"' => token.push('"'),
                        '\\' => token.push('\\'),
                        'n' => token.push('\n'),
                        't' => token.push('\t'),
                        c => token.push(c),
                    }
                } else {
                    token.push(chars[i]);
                }
                i += 1;
            }
            if i >= chars.len() {
                return Err(PortError::InvalidInput {
                    message: "unterminated double quote".to_string(),
                });
            }
            i += 1;
        } else {
            while i < chars.len() && !chars[i].is_ascii_whitespace() {
                token.push(chars[i]);
                i += 1;
            }
        }
        tokens.push(token);
    }
    Ok(tokens)
}
