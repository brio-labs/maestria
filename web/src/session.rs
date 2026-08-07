use web_sys::{Storage, Window};

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Session {
    bearer: String,
}

impl Session {
    pub fn from_browser() -> Self {
        let Some(window) = web_window() else {
            return Self::default();
        };
        let fragment = location_string(window.location().hash());
        let session = fragment
            .strip_prefix('#')
            .and_then(|value| {
                value
                    .split('&')
                    .find_map(|part| part.strip_prefix("session="))
            })
            .map_or_else(String::new, ToOwned::to_owned);
        let storage = window.session_storage().ok().flatten();
        if !session.is_empty() {
            if let Some(storage) = storage.as_ref() {
                let _ = storage.set_item("maestria.studio.bearer", &session);
            }
            let path = location_string(window.location().pathname());
            let query = location_string(window.location().search());
            if let Ok(history) = window.history() {
                let _ = history.replace_state_with_url(
                    &wasm_bindgen::JsValue::NULL,
                    "",
                    Some(&format!("{path}{query}")),
                );
            }
            return Self { bearer: session };
        }
        let bearer = option_string(
            storage.and_then(|value| value.get_item("maestria.studio.bearer").ok().flatten()),
        );
        Self { bearer }
    }
    pub fn bearer(&self) -> &str {
        &self.bearer
    }
    pub fn remembered_notebook() -> Option<u64> {
        storage()
            .and_then(|value| value.get_item("maestria.studio.notebook").ok().flatten())
            .and_then(|value| value.parse().ok())
    }
    pub fn remember_notebook(id: u64) {
        if let Some(value) = storage() {
            let _ = value.set_item("maestria.studio.notebook", &id.to_string());
        }
    }
    pub fn clear_notebook() {
        if let Some(value) = storage() {
            let _ = value.remove_item("maestria.studio.notebook");
        }
    }
}
fn location_string(value: Result<String, wasm_bindgen::JsValue>) -> String {
    let mut result = String::new();
    if let Ok(value) = value {
        result = value;
    }
    result
}

fn option_string(value: Option<String>) -> String {
    let mut result = String::new();
    if let Some(value) = value {
        result = value;
    }
    result
}

fn web_window() -> Option<Window> {
    web_sys::window()
}
fn storage() -> Option<Storage> {
    web_window().and_then(|window| window.session_storage().ok().flatten())
}
