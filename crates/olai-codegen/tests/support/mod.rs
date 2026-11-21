//! Helpers shared by the integration tests.

use std::path::Path;

use swc_common::{FileName, SourceMap, sync::Lrc};
use swc_ecma_parser::lexer::Lexer;
use swc_ecma_parser::{Parser, StringInput, Syntax, TsSyntax};

/// Assert that the generated TypeScript `src` (from `path`) parses: a test that only matches
/// substrings can't tell that a binding named after a reserved word is a syntax error.
pub fn assert_ts_parses(path: &Path, src: &str) {
    let source_map: Lrc<SourceMap> = Default::default();
    let file = source_map.new_source_file(FileName::Anon.into(), src.to_string());
    let lexer = Lexer::new(
        Syntax::Typescript(TsSyntax::default()),
        Default::default(),
        StringInput::from(&*file),
        None,
    );
    let mut parser = Parser::new_from(lexer);
    let parsed = parser.parse_module();
    let mut errors: Vec<_> = parser
        .take_errors()
        .into_iter()
        .map(|e| e.into_kind())
        .collect();
    if let Err(e) = parsed {
        errors.push(e.into_kind());
    }
    assert!(
        errors.is_empty(),
        "{} does not parse as TypeScript: {errors:?}",
        path.display()
    );
}
