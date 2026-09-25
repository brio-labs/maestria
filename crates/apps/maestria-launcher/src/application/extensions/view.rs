use maestria_extensions::{Action, DetailBlock, FormField, FormValue, FormValues, View};
use slint::{ModelRc, SharedString, VecModel};

use crate::{
    ExtensionActionRow, ExtensionChoiceRow, ExtensionFieldRow, ExtensionItemRow, LauncherWindow,
};

pub(super) fn present(ui: &LauncherWindow, view: &View) {
    reset_presentation(ui);
    match view {
        View::List { .. } => present_list(ui, view),
        View::Detail { .. } => present_detail(ui, view),
        View::Form { .. } => present_form(ui, view),
        View::Loading { .. } => present_loading(ui, view),
        View::Error { .. } => present_error(ui, view),
        View::Actions { .. } => present_actions(ui, view),
    }
}

fn reset_presentation(ui: &LauncherWindow) {
    ui.set_extension_items(ModelRc::new(VecModel::default()));
    ui.set_extension_actions(ModelRc::new(VecModel::default()));
    ui.set_extension_fields(ModelRc::new(VecModel::default()));
    ui.set_extension_detail_content("".into());
    ui.set_extension_status_message("".into());
    ui.set_extension_error_message("".into());
    ui.set_extension_selected_item_id("".into());
    ui.set_extension_selected_action_id("".into());
}

fn present_list(ui: &LauncherWindow, view: &View) {
    let View::List {
        title,
        items,
        empty_message,
        actions,
    } = view
    else {
        return;
    };
    ui.set_extension_view_title(title.as_str().into());
    ui.set_extension_status_message(match empty_message.as_deref() {
        Some(message) if !message.is_empty() => message.into(),
        _ => "No matches.".into(),
    });
    ui.set_extension_items(ModelRc::new(VecModel::from(
        items
            .iter()
            .map(|item| ExtensionItemRow {
                id: item.id.as_str().into(),
                title: item.title.as_str().into(),
                subtitle: match item.subtitle.as_deref() {
                    Some(subtitle) if !subtitle.is_empty() => subtitle.into(),
                    _ => "".into(),
                },
            })
            .collect::<Vec<_>>(),
    )));
    ui.set_extension_actions(action_rows(actions));
    ui.set_extension_view("list".into());
}

fn present_detail(ui: &LauncherWindow, view: &View) {
    let View::Detail {
        title,
        blocks,
        actions,
    } = view
    else {
        return;
    };
    ui.set_extension_view_title(title.as_str().into());
    let mut text = String::new();
    let mut properties = Vec::new();
    for block in blocks {
        match block {
            DetailBlock::Text { text: paragraph } => {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(paragraph);
            }
            DetailBlock::Properties { rows } => {
                properties.extend(rows.iter().map(|row| ExtensionItemRow {
                    id: row.label.as_str().into(),
                    title: row.label.as_str().into(),
                    subtitle: row.value.as_str().into(),
                }));
            }
        }
    }
    ui.set_extension_detail_content(text.into());
    ui.set_extension_items(ModelRc::new(VecModel::from(properties)));
    ui.set_extension_actions(action_rows(actions));
    ui.set_extension_view("detail".into());
}

fn present_form(ui: &LauncherWindow, view: &View) {
    let View::Form {
        title,
        fields,
        actions,
    } = view
    else {
        return;
    };
    ui.set_extension_view_title(title.as_str().into());
    ui.set_extension_fields(ModelRc::new(VecModel::from(
        fields.iter().map(field_row).collect::<Vec<_>>(),
    )));
    ui.set_extension_actions(action_rows(actions));
    ui.set_extension_view("form".into());
}

fn present_loading(ui: &LauncherWindow, view: &View) {
    let View::Loading { message } = view else {
        return;
    };
    ui.set_extension_status_message(match message.as_deref() {
        Some(message) if !message.is_empty() => message.into(),
        _ => "Loading extension…".into(),
    });
    ui.set_extension_view("loading".into());
}

fn present_error(ui: &LauncherWindow, view: &View) {
    let View::Error {
        title,
        message,
        actions,
    } = view
    else {
        return;
    };
    ui.set_extension_view_title(match title.as_deref() {
        Some(title) if !title.is_empty() => title.into(),
        _ => "Extension error".into(),
    });
    ui.set_extension_status_message(message.as_str().into());
    ui.set_extension_actions(action_rows(actions));
    ui.set_extension_view("error".into());
}

fn present_actions(ui: &LauncherWindow, view: &View) {
    let View::Actions { title, actions } = view else {
        return;
    };
    ui.set_extension_view_title(title.as_str().into());
    ui.set_extension_actions(action_rows(actions));
    ui.set_extension_view("actions".into());
}

pub(super) fn sync_form_values(ui: &LauncherWindow, view: &View, values: &FormValues) {
    let View::Form { fields, .. } = view else {
        return;
    };
    let rows = fields
        .iter()
        .map(|field| {
            let mut row = field_row(field);
            if let Some(value) = values.get(row.id.as_str()) {
                row.value = match value {
                    FormValue::Text(value) => value.as_str().into(),
                    FormValue::Number(value) => value.to_string().into(),
                    FormValue::Bool(value) => if *value { "true" } else { "false" }.into(),
                    FormValue::Null(()) => SharedString::default(),
                };
            }
            row
        })
        .collect::<Vec<_>>();
    ui.set_extension_fields(ModelRc::new(VecModel::from(rows)));
}

pub(super) fn item_actions(view: &View, item_id: &str) -> Option<ModelRc<ExtensionActionRow>> {
    match view {
        View::List { actions, .. } if item_id.is_empty() => Some(action_rows(actions)),
        View::List { items, .. } => items
            .iter()
            .find(|item| item.id == item_id)
            .map(|item| action_rows(&item.actions)),
        _ => None,
    }
}

pub(super) fn allowed_action(view: &View, action_id: &str, item_id: Option<&str>) -> bool {
    match (view, item_id) {
        (View::List { items, .. }, Some(item_id)) => items.iter().any(|item| {
            item.id == item_id && item.actions.iter().any(|action| action.id == action_id)
        }),
        (View::List { actions, .. }, None)
        | (View::Detail { actions, .. }, None)
        | (View::Form { actions, .. }, None)
        | (View::Error { actions, .. }, None)
        | (View::Actions { actions, .. }, None) => {
            actions.iter().any(|action| action.id == action_id)
        }
        _ => false,
    }
}

fn action_rows(actions: &[Action]) -> ModelRc<ExtensionActionRow> {
    ModelRc::new(VecModel::from(
        actions
            .iter()
            .map(|action| ExtensionActionRow {
                id: action.id.as_str().into(),
                label: action.label.as_str().into(),
            })
            .collect::<Vec<_>>(),
    ))
}

fn field_row(field: &FormField) -> ExtensionFieldRow {
    let (id, label, kind, value, placeholder, choices): (
        &String,
        &String,
        &str,
        SharedString,
        &str,
        Vec<ExtensionChoiceRow>,
    ) = match field {
        FormField::Text {
            id,
            label,
            initial_value,
            placeholder,
            ..
        } => (
            id,
            label,
            "text",
            match initial_value.as_deref() {
                Some(value) if !value.is_empty() => value.into(),
                _ => SharedString::default(),
            },
            match placeholder.as_deref() {
                Some(placeholder) if !placeholder.is_empty() => placeholder,
                _ => "",
            },
            Vec::new(),
        ),
        FormField::Number {
            id,
            label,
            initial_value,
            ..
        } => (
            id,
            label,
            "number",
            initial_value.map_or_else(SharedString::default, |value| value.to_string().into()),
            "",
            Vec::new(),
        ),
        FormField::Select {
            id,
            label,
            initial_value,
            choices,
            ..
        } => (
            id,
            label,
            "select",
            match initial_value.as_deref() {
                Some(value) if !value.is_empty() => value.into(),
                _ => SharedString::default(),
            },
            "",
            choices
                .iter()
                .map(|choice| ExtensionChoiceRow {
                    id: choice.id.as_str().into(),
                    label: choice.label.as_str().into(),
                })
                .collect(),
        ),
        FormField::Checkbox {
            id,
            label,
            initial_value,
        } => (
            id,
            label,
            "checkbox",
            (if *initial_value == Some(true) {
                "true"
            } else {
                "false"
            })
            .into(),
            "",
            Vec::new(),
        ),
        FormField::File { id, label, .. } => {
            (id, label, "file", SharedString::default(), "", Vec::new())
        }
    };
    ExtensionFieldRow {
        id: id.as_str().into(),
        label: label.as_str().into(),
        kind: kind.into(),
        value,
        placeholder: placeholder.into(),
        choices: ModelRc::new(VecModel::from(choices)),
    }
}
