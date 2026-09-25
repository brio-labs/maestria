use std::fmt;

const MAX_INPUT_BYTES: usize = 256;
const MAX_TOKENS: usize = 128;
const MAX_NESTING_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CalculationError {
    InputTooLong,
    TooManyTokens,
    NestingTooDeep,
    InvalidSyntax,
    UnsupportedSyntax,
    DivisionByZero,
    NonFiniteResult,
}

impl fmt::Display for CalculationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InputTooLong => "Expression is too long",
            Self::TooManyTokens => "Expression has too many tokens",
            Self::NestingTooDeep => "Expression is nested too deeply",
            Self::InvalidSyntax => "Incomplete or invalid arithmetic expression",
            Self::UnsupportedSyntax => "Unsupported arithmetic syntax",
            Self::DivisionByZero => "Division by zero is not allowed",
            Self::NonFiniteResult => "Arithmetic result is not finite",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for CalculationError {}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Token {
    Number(f64),
    Plus,
    Minus,
    Star,
    Slash,
    LeftParen,
    RightParen,
}

/// Evaluate one small arithmetic expression, if the input looks arithmetic.
///
/// Ordinary launcher queries return `Ok(None)`. Inputs beginning with `=` are
/// always treated as calculator input, while unprefixed input must contain an
/// ASCII digit and consist only of the calculator character set.
pub fn calculate(input: &str) -> Result<Option<String>, CalculationError> {
    let trimmed = input.trim();
    let explicit = trimmed.starts_with('=');
    if !explicit && !looks_like_arithmetic(trimmed) {
        return Ok(None);
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err(CalculationError::InputTooLong);
    }

    let expression = if explicit {
        trimmed[1..].trim()
    } else {
        trimmed
    };
    let tokens = tokenize(expression)?;
    let mut parser = Parser::new(&tokens);
    let value = parser.parse()?;
    let value = finite(value)?;
    let value = if value == 0.0 { 0.0 } else { value };
    Ok(Some(value.to_string()))
}

fn looks_like_arithmetic(input: &str) -> bool {
    input.chars().any(|character| character.is_ascii_digit())
        && input.chars().all(|character| {
            character.is_ascii_digit()
                || character.is_whitespace()
                || matches!(character, '.' | '+' | '-' | '*' | '/' | '(' | ')')
        })
}

fn tokenize(expression: &str) -> Result<Vec<Token>, CalculationError> {
    let mut characters = expression.char_indices().peekable();
    let mut tokens = Vec::with_capacity(16);
    while let Some((start, character)) = characters.next() {
        let token = match character {
            character if character.is_whitespace() => continue,
            '+' => Token::Plus,
            '-' => Token::Minus,
            '*' => Token::Star,
            '/' => Token::Slash,
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            character if character.is_ascii_digit() || character == '.' => {
                let mut decimal_seen = character == '.';
                let mut digits = usize::from(character.is_ascii_digit());
                while let Some((_, next)) = characters.peek() {
                    if next.is_ascii_digit() {
                        digits += 1;
                    } else if *next == '.' && !decimal_seen {
                        decimal_seen = true;
                    } else {
                        break;
                    }
                    characters.next();
                }
                if digits == 0 {
                    return Err(CalculationError::InvalidSyntax);
                }
                let end = characters
                    .peek()
                    .map_or(expression.len(), |(index, _)| *index);
                let number = expression[start..end]
                    .parse::<f64>()
                    .map_err(|_| CalculationError::InvalidSyntax)?;
                Token::Number(finite(number)?)
            }
            _ => return Err(CalculationError::UnsupportedSyntax),
        };
        if tokens.len() >= MAX_TOKENS {
            return Err(CalculationError::TooManyTokens);
        }
        tokens.push(token);
    }
    Ok(tokens)
}

struct Parser<'a> {
    tokens: &'a [Token],
    position: usize,
}

impl<'a> Parser<'a> {
    fn new(tokens: &'a [Token]) -> Self {
        Self {
            tokens,
            position: 0,
        }
    }

    fn parse(&mut self) -> Result<f64, CalculationError> {
        if self.tokens.is_empty() {
            return Err(CalculationError::InvalidSyntax);
        }
        let value = self.parse_expression(0)?;
        if self.position != self.tokens.len() {
            return Err(CalculationError::InvalidSyntax);
        }
        Ok(value)
    }

    fn parse_expression(&mut self, depth: usize) -> Result<f64, CalculationError> {
        let mut value = self.parse_term(depth)?;
        loop {
            let operation = match self.peek() {
                Some(Token::Plus) => Some(true),
                Some(Token::Minus) => Some(false),
                _ => None,
            };
            let Some(add) = operation else {
                break;
            };
            self.position += 1;
            let right = self.parse_term(depth)?;
            value = if add {
                finite(value + right)?
            } else {
                finite(value - right)?
            };
        }
        Ok(value)
    }

    fn parse_term(&mut self, depth: usize) -> Result<f64, CalculationError> {
        let mut value = self.parse_factor(depth)?;
        loop {
            let operation = match self.peek() {
                Some(Token::Star) => Some(true),
                Some(Token::Slash) => Some(false),
                _ => None,
            };
            let Some(multiply) = operation else {
                break;
            };
            self.position += 1;
            let right = self.parse_factor(depth)?;
            value = if multiply {
                finite(value * right)?
            } else {
                if right == 0.0 {
                    return Err(CalculationError::DivisionByZero);
                }
                finite(value / right)?
            };
        }
        Ok(value)
    }

    fn parse_factor(&mut self, depth: usize) -> Result<f64, CalculationError> {
        match self.peek().copied() {
            Some(Token::Plus) => {
                self.ensure_depth(depth)?;
                self.position += 1;
                self.parse_factor(depth + 1)
            }
            Some(Token::Minus) => {
                self.ensure_depth(depth)?;
                self.position += 1;
                let value = self.parse_factor(depth + 1)?;
                finite(-value)
            }
            Some(Token::Number(number)) => {
                self.position += 1;
                Ok(number)
            }
            Some(Token::LeftParen) => {
                self.ensure_depth(depth)?;
                self.position += 1;
                let value = self.parse_expression(depth + 1)?;
                if !matches!(self.peek(), Some(Token::RightParen)) {
                    return Err(CalculationError::InvalidSyntax);
                }
                self.position += 1;
                Ok(value)
            }
            Some(Token::RightParen) | Some(Token::Star) | Some(Token::Slash) | None => {
                Err(CalculationError::InvalidSyntax)
            }
        }
    }

    fn ensure_depth(&self, depth: usize) -> Result<(), CalculationError> {
        if depth >= MAX_NESTING_DEPTH {
            Err(CalculationError::NestingTooDeep)
        } else {
            Ok(())
        }
    }

    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.position)
    }
}

fn finite(value: f64) -> Result<f64, CalculationError> {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(CalculationError::NonFiniteResult)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_precedence_unary_parentheses_and_decimals() {
        assert_eq!(calculate("2 + 2 * 3"), Ok(Some("8".to_string())));
        assert_eq!(calculate("= -(2 + .5) * 2"), Ok(Some("-5".to_string())));
        assert_eq!(calculate("12."), Ok(Some("12".to_string())));
    }

    #[test]
    fn non_arithmetic_input_is_ignored_but_explicit_input_errors() {
        assert_eq!(calculate("terminal"), Ok(None));
        assert_eq!(calculate("2e3"), Ok(None));
        assert!(matches!(
            calculate("= 2e3"),
            Err(CalculationError::UnsupportedSyntax)
        ));
        assert!(matches!(
            calculate("2 +"),
            Err(CalculationError::InvalidSyntax)
        ));
    }

    #[test]
    fn rejects_zero_division_and_normalizes_negative_zero() {
        assert!(matches!(
            calculate("4 / 0"),
            Err(CalculationError::DivisionByZero)
        ));
        assert_eq!(calculate("-0"), Ok(Some("0".to_string())));
    }

    #[test]
    fn enforces_input_token_and_nesting_limits() {
        assert_eq!(
            calculate(&format!("{}1", "0".repeat(MAX_INPUT_BYTES - 1))),
            Ok(Some("1".to_string()))
        );
        let overlong = "1".repeat(MAX_INPUT_BYTES + 1);
        assert!(matches!(
            calculate(&overlong),
            Err(CalculationError::InputTooLong)
        ));

        let many_tokens = (0..65).map(|_| "1 +").collect::<String>() + "1";
        assert!(matches!(
            calculate(&many_tokens),
            Err(CalculationError::TooManyTokens)
        ));

        let nested = "(".repeat(MAX_NESTING_DEPTH + 1) + "1" + &")".repeat(MAX_NESTING_DEPTH + 1);
        assert!(matches!(
            calculate(&nested),
            Err(CalculationError::NestingTooDeep)
        ));
    }
}
