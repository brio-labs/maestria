use super::*;

pub(crate) fn contract_failure(message: impl Into<String>) -> Box<dyn std::error::Error> {
    Box::new(crate::PortError::internal("contract helper", message))
}

macro_rules! contract_assert {
    ($condition:expr $(,)?) => {
        if !$condition {
            return Err($crate::contract_tests::contract_failure(format!(
                "assertion failed: `{}`",
                stringify!($condition)
            )));
        }
    };
    ($condition:expr, $($arg:tt)+) => {
        if !$condition {
            return Err($crate::contract_tests::contract_failure(format!($($arg)+)));
        }
    };
}

macro_rules! contract_assert_eq {
    ($left:expr, $right:expr $(,)?) => {{
        match (&$left, &$right) {
            (left, right) => {
                if !(*left == *right) {
                    return Err($crate::contract_tests::contract_failure(format!(
                        "assertion failed: `{} == {}`\n  left: {:?}\n right: {:?}",
                        stringify!($left),
                        stringify!($right),
                        left,
                        right
                    )));
                }
            }
        }
    }};
    ($left:expr, $right:expr, $($arg:tt)+) => {{
        match (&$left, &$right) {
            (left, right) => {
                if !(*left == *right) {
                    return Err($crate::contract_tests::contract_failure(format!(
                        "assertion failed: `{} == {}`\n{}\n  left: {:?}\n right: {:?}",
                        stringify!($left),
                        stringify!($right),
                        format_args!($($arg)+),
                        left,
                        right
                    )));
                }
            }
        }
    }};
}

pub(crate) use contract_assert;
pub(crate) use contract_assert_eq;

mod allocator;
mod approval;
mod fixtures;
mod full_text;
mod lifecycle;
mod misc;
mod realm_read_grant;
mod repository;
mod vector;

pub use allocator::assert_id_allocator_contract;
pub use approval::assert_approval_repository_contract;
pub use approval::pending_record;
pub use fixtures::fixture_embedding_identity;
pub use full_text::*;
pub use lifecycle::assert_effect_journal_contract;
pub use misc::*;
pub use realm_read_grant::assert_realm_read_grant_repository_contract;
pub use repository::*;
pub use sillage_test_support::search_budget;
pub use vector::*;
