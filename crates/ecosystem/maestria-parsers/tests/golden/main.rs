#[path = "../common/mod.rs"]
mod common;

mod code;
mod pdf;
mod rejection;
mod text;

use std::error::Error;

fn assert_debug_snapshot<T: std::fmt::Debug>(
    name: &str,
    value: &T,
    function_name: &str,
    file: &str,
    expression: &str,
    assertion_line: u32,
) -> Result<(), Box<dyn Error>> {
    let rendered = format!("{value:#?}");
    insta::_macro_support::assert_snapshot(
        (name.to_owned(), rendered.as_str()).into(),
        insta::_get_workspace_root!().as_path(),
        function_name,
        "golden",
        file,
        assertion_line,
        expression,
    )
}
