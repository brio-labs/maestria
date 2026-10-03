use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

use super::ProtocolError;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub role: Option<ActionRole>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ActionRole {
    Primary,
    Secondary,
    Destructive,
    Submit,
    Cancel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ListItem {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub subtitle: Option<String>,
    #[serde(default)]
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PropertyRow {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum DetailBlock {
    Text { text: String },
    Properties { rows: Vec<PropertyRow> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SelectChoice {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum FormField {
    Text {
        id: String,
        label: String,
        #[serde(default)]
        required: bool,
        #[serde(default)]
        placeholder: Option<String>,
        #[serde(rename = "initialValue", default)]
        initial_value: Option<String>,
        #[serde(rename = "maxLength", default)]
        max_length: Option<usize>,
    },
    Number {
        id: String,
        label: String,
        #[serde(default)]
        required: bool,
        #[serde(default)]
        min: Option<f64>,
        #[serde(default)]
        max: Option<f64>,
        #[serde(default)]
        step: Option<f64>,
        #[serde(rename = "initialValue", default)]
        initial_value: Option<f64>,
    },
    Select {
        id: String,
        label: String,
        #[serde(default)]
        required: bool,
        choices: Vec<SelectChoice>,
        #[serde(rename = "initialValue", default)]
        initial_value: Option<String>,
    },
    Checkbox {
        id: String,
        label: String,
        #[serde(rename = "initialValue", default)]
        initial_value: Option<bool>,
    },
    File {
        id: String,
        label: String,
        #[serde(default)]
        required: bool,
    },
}

impl FormField {
    fn id_and_label(&self) -> (&str, &str) {
        match self {
            Self::Text { id, label, .. }
            | Self::Number { id, label, .. }
            | Self::Select { id, label, .. }
            | Self::Checkbox { id, label, .. }
            | Self::File { id, label, .. } => (id, label),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum View {
    List {
        title: String,
        items: Vec<ListItem>,
        #[serde(rename = "emptyMessage", default)]
        empty_message: Option<String>,
        #[serde(default)]
        actions: Vec<Action>,
    },
    Detail {
        title: String,
        blocks: Vec<DetailBlock>,
        #[serde(default)]
        actions: Vec<Action>,
    },
    Form {
        title: String,
        fields: Vec<FormField>,
        #[serde(default)]
        actions: Vec<Action>,
    },
    Loading {
        #[serde(default)]
        message: Option<String>,
    },
    Error {
        #[serde(default)]
        title: Option<String>,
        message: String,
        #[serde(default)]
        actions: Vec<Action>,
    },
    Actions {
        title: String,
        actions: Vec<Action>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FormValue {
    Text(String),
    Number(f64),
    Bool(bool),
    Null(()),
}

pub type FormValues = BTreeMap<String, FormValue>;

pub(super) fn validate_form_values(values: &FormValues) -> Result<(), ProtocolError> {
    if values.len() > 32 {
        return Err(ProtocolError::Invalid("form value count"));
    }
    for (key, value) in values {
        if !valid_key(key) {
            return Err(ProtocolError::Invalid("form value key"));
        }
        match value {
            FormValue::Text(text) if !bounded(text, 4096) => {
                return Err(ProtocolError::Invalid("form value text"));
            }
            FormValue::Number(number) if !number.is_finite() => {
                return Err(ProtocolError::Invalid("form value number"));
            }
            _ => {}
        }
    }
    Ok(())
}

pub(super) fn bounded(text: &str, max: usize) -> bool {
    text.encode_utf16().take(max + 1).count() <= max
}

pub(super) fn valid_key(key: &str) -> bool {
    !key.is_empty()
        && bounded(key, 64)
        && key
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn validate_actions(actions: &[Action]) -> Result<(), ProtocolError> {
    if actions.len() > 32 {
        return Err(ProtocolError::Invalid("action count"));
    }
    let mut ids = BTreeSet::new();
    for action in actions {
        if !valid_key(&action.id)
            || !bounded(&action.label, 120)
            || action.label.trim().is_empty()
            || !ids.insert(action.id.as_str())
        {
            return Err(ProtocolError::Invalid("action ID or label"));
        }
    }
    Ok(())
}

pub fn validate_view(view: &View) -> Result<(), ProtocolError> {
    match view {
        View::List {
            title,
            items,
            empty_message,
            actions,
        } => validate_list(title, items, empty_message.as_deref(), actions),
        View::Detail {
            title,
            blocks,
            actions,
        } => validate_detail(title, blocks, actions),
        View::Form {
            title,
            fields,
            actions,
        } => validate_form(title, fields, actions),
        View::Loading { message } => {
            if message.as_deref().is_some_and(|text| !bounded(text, 4096)) {
                return Err(ProtocolError::Invalid("loading message"));
            }
            Ok(())
        }
        View::Error {
            title,
            message,
            actions,
        } => {
            if title.as_deref().is_some_and(|title| !bounded(title, 120)) || !bounded(message, 4096)
            {
                return Err(ProtocolError::Invalid("error text"));
            }
            validate_actions(actions)
        }
        View::Actions { title, actions } => {
            if !bounded_title(title) {
                return Err(ProtocolError::Invalid("action panel title"));
            }
            validate_actions(actions)
        }
    }
}

fn bounded_title(title: &str) -> bool {
    bounded(title, 120) && !title.trim().is_empty()
}

fn validate_list(
    title: &str,
    items: &[ListItem],
    empty_message: Option<&str>,
    actions: &[Action],
) -> Result<(), ProtocolError> {
    if !bounded_title(title)
        || items.len() > 100
        || empty_message.is_some_and(|text| !bounded(text, 4096))
    {
        return Err(ProtocolError::Invalid("list size or text"));
    }
    validate_actions(actions)?;
    let mut ids = BTreeSet::new();
    for item in items {
        if !valid_key(&item.id)
            || !ids.insert(item.id.as_str())
            || !bounded_title(&item.title)
            || item
                .subtitle
                .as_deref()
                .is_some_and(|text| !bounded(text, 4096))
        {
            return Err(ProtocolError::Invalid("list item ID or text"));
        }
        validate_actions(&item.actions)?;
    }
    Ok(())
}

fn validate_detail(
    title: &str,
    blocks: &[DetailBlock],
    actions: &[Action],
) -> Result<(), ProtocolError> {
    if !bounded_title(title) || blocks.len() > 32 {
        return Err(ProtocolError::Invalid("detail title or block count"));
    }
    validate_actions(actions)?;
    for block in blocks {
        match block {
            DetailBlock::Text { text } if !bounded(text, 4096) => {
                return Err(ProtocolError::Invalid("detail text"));
            }
            DetailBlock::Properties { rows }
                if rows.len() > 100
                    || rows
                        .iter()
                        .any(|row| !bounded(&row.label, 120) || !bounded(&row.value, 4096)) =>
            {
                return Err(ProtocolError::Invalid("detail properties"));
            }
            _ => {}
        }
    }
    Ok(())
}

fn validate_form(
    title: &str,
    fields: &[FormField],
    actions: &[Action],
) -> Result<(), ProtocolError> {
    if !bounded_title(title) || fields.len() > 32 {
        return Err(ProtocolError::Invalid("form title or field count"));
    }
    validate_actions(actions)?;
    let mut ids = BTreeSet::new();
    for field in fields {
        let (id, label) = field.id_and_label();
        if !valid_key(id) || !ids.insert(id) || !bounded_title(label) {
            return Err(ProtocolError::Invalid("form field ID or label"));
        }
        validate_form_field(field)?;
    }
    Ok(())
}

fn validate_form_field(field: &FormField) -> Result<(), ProtocolError> {
    match field {
        FormField::Text {
            placeholder,
            initial_value,
            max_length,
            ..
        } => {
            if placeholder
                .as_deref()
                .is_some_and(|text| !bounded(text, 4096))
                || initial_value
                    .as_deref()
                    .is_some_and(|text| !bounded(text, 4096))
                || max_length.is_some_and(|length| length > 4096)
            {
                return Err(ProtocolError::Invalid("form text field"));
            }
        }
        FormField::Number {
            min,
            max,
            step,
            initial_value,
            ..
        } => {
            if [min, max, step, initial_value]
                .into_iter()
                .flatten()
                .any(|number| !number.is_finite())
                || step.is_some_and(|value| value <= 0.0)
                || min.zip(*max).is_some_and(|(lo, hi)| lo > hi)
            {
                return Err(ProtocolError::Invalid("form number field"));
            }
        }
        FormField::Select {
            choices,
            initial_value,
            ..
        } => {
            if choices.is_empty()
                || choices.len() > 100
                || initial_value
                    .as_ref()
                    .is_some_and(|initial| !choices.iter().any(|choice| &choice.id == initial))
            {
                return Err(ProtocolError::Invalid("form choices"));
            }
            let mut seen = BTreeSet::new();
            if choices.iter().any(|choice| {
                !valid_key(&choice.id)
                    || !seen.insert(choice.id.as_str())
                    || !bounded(&choice.label, 120)
            }) {
                return Err(ProtocolError::Invalid("form choice ID or label"));
            }
        }
        _ => {}
    }
    Ok(())
}
