use super::{CalculationError, MAX_TOKENS, finite};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dimension {
    Length,
    Mass,
    Temperature,
    Duration,
    DigitalStorage,
}

#[derive(Clone, Copy)]
enum TemperatureUnit {
    Celsius,
    Fahrenheit,
    Kelvin,
}

#[derive(Clone, Copy)]
enum Unit {
    Linear { dimension: Dimension, scale: f64 },
    Temperature(TemperatureUnit),
}

impl Unit {
    fn dimension(self) -> Dimension {
        match self {
            Self::Linear { dimension, .. } => dimension,
            Self::Temperature(_) => Dimension::Temperature,
        }
    }
}

pub(super) fn looks_like_conversion(input: &str) -> bool {
    let mut tokens = input.split_whitespace();
    let Some(value) = tokens.next() else {
        return false;
    };
    let number_intent = is_number_intent(value);
    let mut token_count = 1;
    let mut has_separator = false;
    let mut has_known_unit = false;
    for token in tokens {
        token_count += 1;
        if token.eq_ignore_ascii_case("to") {
            has_separator = true;
        } else if resolve_unit(token).is_some() {
            has_known_unit = true;
        }
    }
    has_separator
        && ((number_intent && (has_known_unit || token_count < 4))
            || (!number_intent && has_known_unit && token_count <= 4))
}

pub(super) fn convert(input: &str) -> Result<f64, CalculationError> {
    let token_count = input.split_whitespace().count();
    if token_count > MAX_TOKENS {
        return Err(CalculationError::TooManyTokens);
    }
    if token_count != 4 {
        return Err(CalculationError::InvalidConversionSyntax);
    }

    let mut tokens = input.split_whitespace();
    let value_token = tokens
        .next()
        .ok_or(CalculationError::InvalidConversionSyntax)?;
    let source_token = tokens
        .next()
        .ok_or(CalculationError::InvalidConversionSyntax)?;
    let separator = tokens
        .next()
        .ok_or(CalculationError::InvalidConversionSyntax)?;
    let target_token = tokens
        .next()
        .ok_or(CalculationError::InvalidConversionSyntax)?;
    if !separator.eq_ignore_ascii_case("to") || !is_number_literal(value_token) {
        return Err(CalculationError::InvalidConversionSyntax);
    }

    let value = value_token
        .parse::<f64>()
        .map_err(|_| CalculationError::InvalidConversionSyntax)?;
    let value = finite(value)?;
    let source = resolve_unit(source_token).ok_or(CalculationError::UnsupportedUnit)?;
    let target = resolve_unit(target_token).ok_or(CalculationError::UnsupportedUnit)?;
    if source.dimension() != target.dimension() {
        return Err(CalculationError::IncompatibleUnits);
    }

    match (source, target) {
        (
            Unit::Linear {
                scale: source_scale,
                ..
            },
            Unit::Linear {
                scale: target_scale,
                ..
            },
        ) => finite(value * (source_scale / target_scale)),
        (Unit::Temperature(source), Unit::Temperature(target)) => {
            let celsius = match source {
                TemperatureUnit::Celsius => value,
                TemperatureUnit::Fahrenheit => (value - 32.0) * (5.0 / 9.0),
                TemperatureUnit::Kelvin => value - 273.15,
            };
            let converted = match target {
                TemperatureUnit::Celsius => celsius,
                TemperatureUnit::Fahrenheit => celsius * (9.0 / 5.0) + 32.0,
                TemperatureUnit::Kelvin => celsius + 273.15,
            };
            finite(converted)
        }
        _ => Err(CalculationError::IncompatibleUnits),
    }
}

fn is_number_intent(token: &str) -> bool {
    let unsigned = token
        .strip_prefix('+')
        .or_else(|| token.strip_prefix('-'))
        .map_or(token, |unsigned| unsigned);
    unsigned
        .as_bytes()
        .first()
        .is_some_and(|byte| byte.is_ascii_digit() || *byte == b'.')
        || matches_special_float(unsigned)
}

fn is_number_literal(token: &str) -> bool {
    let bytes = token.as_bytes();
    let mut index = if bytes
        .first()
        .is_some_and(|byte| matches!(*byte, b'+' | b'-'))
    {
        1
    } else {
        0
    };
    let mut digits = 0;
    while bytes.get(index).is_some_and(u8::is_ascii_digit) {
        digits += 1;
        index += 1;
    }
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            digits += 1;
            index += 1;
        }
    }
    if digits == 0 {
        return matches_special_float(&token[index..]);
    }
    if bytes
        .get(index)
        .is_some_and(|byte| matches!(*byte, b'e' | b'E'))
    {
        index += 1;
        if bytes
            .get(index)
            .is_some_and(|byte| matches!(*byte, b'+' | b'-'))
        {
            index += 1;
        }
        let exponent_start = index;
        while bytes.get(index).is_some_and(u8::is_ascii_digit) {
            index += 1;
        }
        if index == exponent_start {
            return false;
        }
    }
    index == bytes.len()
}

fn matches_special_float(value: &str) -> bool {
    ["nan", "inf", "infinity"]
        .iter()
        .any(|special| value.eq_ignore_ascii_case(special))
}

fn resolve_unit(input: &str) -> Option<Unit> {
    let linear = |dimension, scale| Unit::Linear { dimension, scale };

    if matches(input, &["m", "meter", "meters", "metre", "metres"]) {
        return Some(linear(Dimension::Length, 1.0));
    }
    if matches(
        input,
        &["km", "kilometer", "kilometers", "kilometre", "kilometres"],
    ) {
        return Some(linear(Dimension::Length, 1_000.0));
    }
    if matches(
        input,
        &[
            "cm",
            "centimeter",
            "centimeters",
            "centimetre",
            "centimetres",
        ],
    ) {
        return Some(linear(Dimension::Length, 0.01));
    }
    if matches(
        input,
        &[
            "mm",
            "millimeter",
            "millimeters",
            "millimetre",
            "millimetres",
        ],
    ) {
        return Some(linear(Dimension::Length, 0.001));
    }
    if matches(input, &["mi", "mile", "miles"]) {
        return Some(linear(Dimension::Length, 1_609.344));
    }
    if matches(input, &["yd", "yard", "yards"]) {
        return Some(linear(Dimension::Length, 0.9144));
    }
    if matches(input, &["ft", "foot", "feet"]) {
        return Some(linear(Dimension::Length, 0.3048));
    }
    if matches(input, &["in", "inch", "inches"]) {
        return Some(linear(Dimension::Length, 0.0254));
    }

    if matches(input, &["kg", "kilogram", "kilograms"]) {
        return Some(linear(Dimension::Mass, 1.0));
    }
    if matches(input, &["g", "gram", "grams"]) {
        return Some(linear(Dimension::Mass, 0.001));
    }
    if matches(input, &["mg", "milligram", "milligrams"]) {
        return Some(linear(Dimension::Mass, 0.000_001));
    }
    if matches(input, &["lb", "lbs", "pound", "pounds"]) {
        return Some(linear(Dimension::Mass, 0.453_592_37));
    }
    if matches(input, &["oz", "ounce", "ounces"]) {
        return Some(linear(Dimension::Mass, 0.028_349_523_125));
    }
    if matches(input, &["t", "tonne", "tonnes"]) {
        return Some(linear(Dimension::Mass, 1_000.0));
    }

    if matches(input, &["c", "°c", "degc", "celsius"]) {
        return Some(Unit::Temperature(TemperatureUnit::Celsius));
    }
    if matches(input, &["f", "°f", "degf", "fahrenheit"]) {
        return Some(Unit::Temperature(TemperatureUnit::Fahrenheit));
    }
    if matches(input, &["k", "°k", "degk", "kelvin"]) {
        return Some(Unit::Temperature(TemperatureUnit::Kelvin));
    }

    if matches(input, &["ns", "nanosecond", "nanoseconds"]) {
        return Some(linear(Dimension::Duration, 0.000_000_001));
    }
    if matches(input, &["us", "µs", "microsecond", "microseconds"]) {
        return Some(linear(Dimension::Duration, 0.000_001));
    }
    if matches(input, &["ms", "millisecond", "milliseconds"]) {
        return Some(linear(Dimension::Duration, 0.001));
    }
    if matches(input, &["s", "sec", "second", "seconds"]) {
        return Some(linear(Dimension::Duration, 1.0));
    }
    if matches(input, &["min", "mins", "minute", "minutes"]) {
        return Some(linear(Dimension::Duration, 60.0));
    }
    if matches(input, &["h", "hr", "hrs", "hour", "hours"]) {
        return Some(linear(Dimension::Duration, 3_600.0));
    }
    if matches(input, &["d", "day", "days"]) {
        return Some(linear(Dimension::Duration, 86_400.0));
    }
    if matches(input, &["wk", "week", "weeks"]) {
        return Some(linear(Dimension::Duration, 604_800.0));
    }

    digital_unit(input).map(|scale| linear(Dimension::DigitalStorage, scale))
}

fn digital_unit(input: &str) -> Option<f64> {
    let symbol_scale = match input {
        "b" => Some(1.0),
        "B" => Some(8.0),
        "kb" => Some(1_000.0),
        "kB" | "KB" => Some(8_000.0),
        "mb" | "Mb" => Some(1_000_000.0),
        "MB" => Some(8_000_000.0),
        "gb" | "Gb" => Some(1_000_000_000.0),
        "GB" => Some(8_000_000_000.0),
        "tb" | "Tb" => Some(1_000_000_000_000.0),
        "TB" => Some(8_000_000_000_000.0),
        "Kib" => Some(1_024.0),
        "Mib" => Some(1_048_576.0),
        "Gib" => Some(1_073_741_824.0),
        "Tib" => Some(1_099_511_627_776.0),
        "KiB" => Some(8_192.0),
        "MiB" => Some(8_388_608.0),
        "GiB" => Some(8_589_934_592.0),
        "TiB" => Some(8_796_093_022_208.0),
        _ => None,
    };
    if symbol_scale.is_some() {
        return symbol_scale;
    }

    if matches(input, &["bit", "bits"]) {
        return Some(1.0);
    }
    if matches(input, &["byte", "bytes"]) {
        return Some(8.0);
    }
    if matches(input, &["kilobyte", "kilobytes"]) {
        return Some(8_000.0);
    }
    if matches(input, &["megabyte", "megabytes"]) {
        return Some(8_000_000.0);
    }
    if matches(input, &["gigabyte", "gigabytes"]) {
        return Some(8_000_000_000.0);
    }
    if matches(input, &["terabyte", "terabytes"]) {
        return Some(8_000_000_000_000.0);
    }
    if matches(input, &["kbit", "kilobit", "kilobits"]) {
        return Some(1_000.0);
    }
    if matches(input, &["mbit", "megabit", "megabits"]) {
        return Some(1_000_000.0);
    }
    if matches(input, &["gbit", "gigabit", "gigabits"]) {
        return Some(1_000_000_000.0);
    }
    if matches(input, &["tbit", "terabit", "terabits"]) {
        return Some(1_000_000_000_000.0);
    }
    if matches(input, &["kibit", "kibibit", "kibibits"]) {
        return Some(1_024.0);
    }
    if matches(input, &["mibit", "mebibit", "mebibits"]) {
        return Some(1_048_576.0);
    }
    if matches(input, &["gibit", "gibibit", "gibibits"]) {
        return Some(1_073_741_824.0);
    }
    if matches(input, &["tibit", "tebibit", "tebibits"]) {
        return Some(1_099_511_627_776.0);
    }
    if matches(input, &["kibibyte", "kibibytes"]) {
        return Some(8_192.0);
    }
    if matches(input, &["mebibyte", "mebibytes"]) {
        return Some(8_388_608.0);
    }
    if matches(input, &["gibibyte", "gibibytes"]) {
        return Some(8_589_934_592.0);
    }
    if matches(input, &["tebibyte", "tebibytes"]) {
        return Some(8_796_093_022_208.0);
    }
    None
}

fn matches(input: &str, choices: &[&str]) -> bool {
    choices
        .iter()
        .any(|choice| input.eq_ignore_ascii_case(choice))
}
