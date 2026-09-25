use std::collections::BTreeSet;

use maestria_extensions::{FormField, FormValue, FormValues, View};
use thiserror::Error;

#[derive(Debug, Error)]
pub(super) enum FormInputError {
    #[error("there is no active extension form")]
    NotForm,
    #[error("unknown extension form field {0}")]
    UnknownField(String),
    #[error("invalid value for extension form field {0}")]
    Invalid(String),
    #[error("extension form contains an unselected file {0}")]
    UnselectedFile(String),
}

pub(super) fn initial_values(view: &View) -> FormValues {
    let mut values = FormValues::new();
    let View::Form { fields, .. } = view else {
        return values;
    };
    for field in fields {
        let initial = match field {
            FormField::Text {
                initial_value: Some(text),
                ..
            }
            | FormField::Select {
                initial_value: Some(text),
                ..
            } => Some(FormValue::Text(text.clone())),
            FormField::Number {
                initial_value: Some(number),
                ..
            } => Some(FormValue::Number(*number)),
            FormField::Checkbox { initial_value, .. } => {
                Some(FormValue::Bool(*initial_value == Some(true)))
            }
            _ => None,
        };
        if let Some(initial) = initial {
            values.insert(form_field_id(field).to_owned(), initial);
        }
    }
    values
}

pub(super) fn set_user_value(
    view: &View,
    values: &mut FormValues,
    field_id: &str,
    raw: &str,
) -> Result<(), FormInputError> {
    let field = field(view, field_id)?;
    if matches!(field, FormField::File { .. }) {
        return Err(FormInputError::UnselectedFile(field_id.to_owned()));
    }
    let parsed = match field {
        FormField::Text { .. } | FormField::Select { .. } => FormValue::Text(raw.to_owned()),
        FormField::Number { .. } if raw.is_empty() => FormValue::Null(()),
        FormField::Number { .. } => FormValue::Number(
            raw.parse()
                .map_err(|_| FormInputError::Invalid(field_id.to_owned()))?,
        ),
        FormField::Checkbox { .. } if raw == "true" => FormValue::Bool(true),
        FormField::Checkbox { .. } if raw == "false" => FormValue::Bool(false),
        _ => return Err(FormInputError::Invalid(field_id.to_owned())),
    };
    validate_value(field, &parsed, &BTreeSet::new())?;
    values.insert(field_id.to_owned(), parsed);
    Ok(())
}

pub(super) fn set_host_file(
    view: &View,
    values: &mut FormValues,
    field_id: &str,
    selection_id: &str,
) -> Result<(), FormInputError> {
    if !matches!(field(view, field_id)?, FormField::File { .. }) || selection_id.is_empty() {
        return Err(FormInputError::Invalid(field_id.to_owned()));
    }
    values.insert(
        field_id.to_owned(),
        FormValue::Text(selection_id.to_owned()),
    );
    Ok(())
}

pub(super) fn validate_submission(
    view: &View,
    values: &FormValues,
    selected_file_ids: &BTreeSet<String>,
) -> Result<(), FormInputError> {
    let View::Form { fields, .. } = view else {
        return Err(FormInputError::NotForm);
    };
    if values.len() > fields.len() || values.len() > 32 {
        return Err(FormInputError::Invalid("field count".into()));
    }
    for (id, value) in values {
        let field = fields
            .iter()
            .find(|field| form_field_id(field) == id)
            .ok_or_else(|| FormInputError::UnknownField(id.clone()))?;
        validate_value(field, value, selected_file_ids)?;
    }
    for field in fields {
        let id = form_field_id(field);
        if required(field) && !values.contains_key(id) {
            return Err(FormInputError::Invalid(id.to_owned()));
        }
    }
    Ok(())
}

pub(super) fn is_file_field(view: &View, id: &str) -> bool {
    matches!(field(view, id), Ok(FormField::File { .. }))
}

fn field<'a>(view: &'a View, id: &str) -> Result<&'a FormField, FormInputError> {
    let View::Form { fields, .. } = view else {
        return Err(FormInputError::NotForm);
    };
    fields
        .iter()
        .find(|field| form_field_id(field) == id)
        .ok_or_else(|| FormInputError::UnknownField(id.to_owned()))
}

fn form_field_id(field: &FormField) -> &str {
    match field {
        FormField::Text { id, .. }
        | FormField::Number { id, .. }
        | FormField::Select { id, .. }
        | FormField::Checkbox { id, .. }
        | FormField::File { id, .. } => id,
    }
}

fn required(field: &FormField) -> bool {
    match field {
        FormField::Text { required, .. }
        | FormField::Number { required, .. }
        | FormField::Select { required, .. }
        | FormField::File { required, .. } => *required,
        FormField::Checkbox { .. } => false,
    }
}

fn validate_value(
    field: &FormField,
    value: &FormValue,
    selected_file_ids: &BTreeSet<String>,
) -> Result<(), FormInputError> {
    let valid = match (field, value) {
        (
            FormField::Text {
                required,
                max_length,
                ..
            },
            FormValue::Text(text),
        ) => {
            let limit = match max_length {
                Some(limit) => *limit,
                None => 4096,
            };
            (!required || !text.is_empty()) && text.encode_utf16().count() <= limit
        }
        (FormField::Number { min, max, .. }, FormValue::Number(number)) => {
            number.is_finite()
                && min.is_none_or(|min| *number >= min)
                && max.is_none_or(|max| *number <= max)
        }
        (
            FormField::Select {
                required, choices, ..
            },
            FormValue::Text(id),
        ) => (!required && id.is_empty()) || choices.iter().any(|choice| choice.id == *id),
        (FormField::Checkbox { .. }, FormValue::Bool(_)) => true,
        (FormField::File { id, .. }, FormValue::Text(selection_id)) => {
            if !selected_file_ids.contains(selection_id) {
                return Err(FormInputError::UnselectedFile(id.clone()));
            }
            true
        }
        (
            FormField::Number {
                required: false, ..
            },
            FormValue::Null(()),
        ) => true,
        (
            FormField::Text {
                required: false, ..
            },
            FormValue::Null(()),
        ) => true,
        (
            FormField::Select {
                required: false, ..
            },
            FormValue::Null(()),
        ) => true,
        (
            FormField::File {
                required: false, ..
            },
            FormValue::Null(()),
        ) => true,
        _ => false,
    };
    if !valid {
        return Err(FormInputError::Invalid(form_field_id(field).to_owned()));
    }
    Ok(())
}
