use std::fmt;

mod conversion;

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
    InvalidConversionSyntax,
    UnsupportedUnit,
    IncompatibleUnits,
    DivisionByZero,
    NonFiniteResult,
}

impl fmt::Display for CalculationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::InputTooLong => "Calculator input is too long",
            Self::TooManyTokens => "Calculator input has too many tokens",
            Self::NestingTooDeep => "Expression is nested too deeply",
            Self::InvalidSyntax => "Incomplete or invalid arithmetic expression",
            Self::UnsupportedSyntax => "Unsupported arithmetic syntax",
            Self::InvalidConversionSyntax => {
                "Use '<value> <source unit> to <target unit>' for conversions"
            }
            Self::UnsupportedUnit => {
                "Unsupported unit; examples include km, mi, kg, lb, C, F, h, min, GB, and MB"
            }
            Self::IncompatibleUnits => "Conversion units must measure the same quantity",
            Self::DivisionByZero => "Division by zero is not allowed",
            Self::NonFiniteResult => "Result is not finite",
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

/// Evaluate a small arithmetic expression or supported unit conversion.
///
/// Ordinary launcher queries return `Ok(None)`. Inputs beginning with `=` are
/// always treated as calculator input; unprefixed input is handled only when
/// it looks like arithmetic or a conversion.
pub fn calculate(input: &str) -> Result<Option<String>, CalculationError> {
    let trimmed = input.trim();
    let explicit = trimmed.starts_with('=');
    let expression = if explicit {
        trimmed[1..].trim()
    } else {
        trimmed
    };
    let is_conversion = conversion::looks_like_conversion(expression);
    if !explicit && !is_conversion && !looks_like_arithmetic(trimmed) {
        return Ok(None);
    }
    if input.len() > MAX_INPUT_BYTES {
        return Err(CalculationError::InputTooLong);
    }
    if is_conversion {
        return Ok(Some(format_result(conversion::convert(expression)?)?));
    }

    let tokens = tokenize(expression)?;
    let mut parser = Parser::new(&tokens);
    let value = parser.parse()?;
    Ok(Some(format_result(value)?))
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

fn format_result(value: f64) -> Result<String, CalculationError> {
    let value = finite(value)?;
    let value = if value == 0.0 { 0.0 } else { value };
    Ok(value.to_string())
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

    #[test]
    fn converts_supported_units_across_all_dimensions() -> Result<(), Box<dyn std::error::Error>> {
        let miles = calculate("10 km to mi")?
            .ok_or_else(|| std::io::Error::other("Expected a unit conversion result"))?
            .parse::<f64>()?;
        assert!((miles - 6.213_711_922_373_339).abs() < 1e-12);
        assert_eq!(calculate("1 kg to g"), Ok(Some("1000".to_string())));
        assert_eq!(calculate("2 hours to minutes"), Ok(Some("120".to_string())));
        assert_eq!(calculate("1 GB to MB"), Ok(Some("1000".to_string())));
        assert_eq!(calculate("1 MiB to bit"), Ok(Some("8388608".to_string())));
        assert_eq!(calculate("1 Kib to bit"), Ok(Some("1024".to_string())));
        Ok(())
    }

    #[test]
    fn conversion_temperature_offsets_and_zero_are_preserved() {
        assert_eq!(calculate("32 F to C"), Ok(Some("0".to_string())));
        assert_eq!(calculate("0 C to F"), Ok(Some("32".to_string())));
        assert_eq!(calculate("273.15 K to C"), Ok(Some("0".to_string())));
        assert_eq!(calculate("-0 C to F"), Ok(Some("32".to_string())));
    }

    #[test]
    fn conversion_errors_are_typed_and_ordinary_queries_are_ignored() {
        assert_eq!(calculate("terminal"), Ok(None));
        assert_eq!(calculate("search for 10 km to mi"), Ok(None));
        assert_eq!(calculate("10 things to do"), Ok(None));
        assert!(matches!(
            calculate("10 km to"),
            Err(CalculationError::InvalidConversionSyntax)
        ));
        assert!(matches!(
            calculate("value km to mi"),
            Err(CalculationError::InvalidConversionSyntax)
        ));
        assert!(matches!(
            calculate("10 parsecs to km"),
            Err(CalculationError::UnsupportedUnit)
        ));
        assert!(matches!(
            calculate("10 km to kg"),
            Err(CalculationError::IncompatibleUnits)
        ));
    }

    #[test]
    fn conversions_reject_non_finite_values_and_results() {
        assert!(matches!(
            calculate("1e309 km to mi"),
            Err(CalculationError::NonFiniteResult)
        ));
        assert!(matches!(
            calculate("1e308 km to mm"),
            Err(CalculationError::NonFiniteResult)
        ));
    }

    #[test]
    fn conversions_use_the_existing_input_byte_limit() {
        let expression = "1 km to m";
        let at_limit = format!(
            "{}{}",
            " ".repeat(MAX_INPUT_BYTES - expression.len()),
            expression
        );
        assert_eq!(calculate(&at_limit), Ok(Some("1000".to_string())));

        let over_limit = format!(" {at_limit}");
        assert_eq!(calculate(&over_limit), Err(CalculationError::InputTooLong));
    }
}
