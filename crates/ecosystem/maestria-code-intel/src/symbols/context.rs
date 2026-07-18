use crate::SymbolMarkers;
use crate::identity::RepositoryIdentity;
use crate::symbols::markers::{attr_bench, attr_test};
use syn::Attribute;

#[derive(Debug, Clone)]
pub(crate) struct FileContext<'a> {
    pub(crate) package: &'a str,
    pub(crate) target: &'a str,
    pub(crate) relative_path: String,
    pub(crate) identity: &'a RepositoryIdentity,
    pub(crate) parser_generation: &'a str,
    pub(crate) file_markers: SymbolMarkers,
    pub(crate) is_test_target: bool,
    pub(crate) is_bench_target: bool,
}

impl<'a> FileContext<'a> {
    pub(crate) fn nested(&self, attrs: &[Attribute]) -> Self {
        Self {
            is_test_target: self.is_test_target || attr_test(attrs),
            is_bench_target: self.is_bench_target || attr_bench(attrs),
            ..self.clone()
        }
    }
}
