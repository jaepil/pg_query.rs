use pg_query::{parse_with_options, Diagnostic, Error, NodeEnum, ParseMode, ParseOptions};

fn legacy() -> ParseOptions {
    ParseOptions { standard_conforming_strings: false, ..ParseOptions::default() }
}

fn literal(sql: &str, options: ParseOptions) -> (String, Vec<Diagnostic>) {
    let outcome = parse_with_options(sql, options);
    let result = outcome.result.unwrap();
    let NodeEnum::SelectStmt(select) = result.protobuf.stmts[0].stmt.as_ref().unwrap().node.as_ref().unwrap() else { panic!("SELECT") };
    let NodeEnum::ResTarget(target) = select.target_list[0].node.as_ref().unwrap() else { panic!("target") };
    let NodeEnum::AConst(value) = target.val.as_ref().unwrap().node.as_ref().unwrap() else { panic!("constant") };
    let Some(pg_query::protobuf::a_const::Val::Sval(value)) = &value.val else { panic!("text constant") };
    (value.sval.clone(), outcome.diagnostics)
}

// Values and diagnostic fields were independently captured from PostgreSQL 18.4.
#[test]
fn scanner_options_preserve_values_and_per_literal_warning_fields() {
    assert_eq!(literal(r"SELECT 'a\nb'", ParseOptions::default()), (r"a\nb".into(), vec![]));
    for (sql, value, message, hint) in [
        (r"SELECT 'a\nb'", "a\nb", "nonstandard use of escape in a string literal", r"Use the escape string syntax for escapes, e.g., E'\r\n'."),
        (r"SELECT 'a\\b'", r"a\b", r"nonstandard use of \\ in a string literal", r"Use the escape string syntax for backslashes, e.g., E'\\'."),
        (
            r"SELECT 'a\'b'",
            "a'b",
            r"nonstandard use of \' in a string literal",
            "Use '' to write quotes in strings, or use the escape string syntax (E'...').",
        ),
        (
            r"SELECT '\t\101\x42\u0043\U00000044'",
            "\tABCD",
            "nonstandard use of escape in a string literal",
            r"Use the escape string syntax for escapes, e.g., E'\r\n'.",
        ),
    ] {
        let (parsed, warnings) = literal(sql, legacy());
        assert_eq!(parsed, value);
        let [warning] = warnings.as_slice() else { panic!("one warning per literal: {warnings:?}") };
        assert_eq!(warning.severity, 19);
        assert_eq!(warning.sqlstate, "22P06");
        assert_eq!(warning.message, message);
        assert_eq!(warning.detail, None);
        assert_eq!(warning.hint.as_deref(), Some(hint));
    }
    let outcome = parse_with_options(r"SELECT 'a\nb', 'c\td'", legacy());
    assert!(outcome.result.is_ok());
    assert_eq!(outcome.diagnostics.len(), 2);
    let options = ParseOptions { escape_string_warning: false, ..legacy() };
    assert_eq!(literal(r"SELECT 'a\nb'", options), ("a\nb".into(), vec![]));
    assert_eq!(literal(r"SELECT $$a\nb$$", legacy()), (r"a\nb".into(), vec![]));
}

#[test]
fn original_error_codes_and_fields_survive_parser_failures() {
    for (sql, options, state, message, detail, hint) in [
        (
            r"SELECT U&'d\0061t'",
            legacy(),
            "0A000",
            "unsafe use of string constant with Unicode escapes",
            Some("String constants with Unicode escapes cannot be used when \"standard_conforming_strings\" is off."),
            None,
        ),
        (
            r"SELECT 'a\'b'",
            ParseOptions { backslash_quote: false, ..legacy() },
            "22P06",
            r"unsafe use of \' in a string literal",
            None,
            Some(r"Use '' to write quotes in strings. \' is insecure in client-only encodings."),
        ),
        (
            "SELECT sum(1) OVER (ROWS BETWEEN UNBOUNDED FOLLOWING AND CURRENT ROW)",
            ParseOptions::default(),
            "42P20",
            "frame start cannot be UNBOUNDED FOLLOWING",
            None,
            None,
        ),
    ] {
        let outcome = parse_with_options(sql, options);
        let Error::ParseDiagnostic(error) = outcome.result.unwrap_err() else { panic!("structured error") };
        assert_eq!(error.sqlstate, state);
        assert_eq!(error.message, message);
        assert_eq!(error.detail.as_deref(), detail);
        assert_eq!(error.hint.as_deref(), hint);
        assert!(error.cursor_position > 0);
        assert_eq!(literal(r"SELECT 'a\nb'", ParseOptions::default()), (r"a\nb".into(), vec![]));
    }
}

#[test]
fn warnings_preceding_errors_are_retained_in_order() {
    let outcome = parse_with_options(r"SELECT 'a\nb', (", legacy());
    assert!(outcome.result.is_err());
    assert_eq!(outcome.diagnostics.iter().map(|diagnostic| diagnostic.severity).collect::<Vec<_>>(), [19, 21]);
    assert_eq!(outcome.diagnostics[0].sqlstate, "22P06");
    assert_eq!(outcome.diagnostics[1].sqlstate, "42601");
}

#[test]
fn expression_modes_and_parallel_calls_keep_scanner_options_local() {
    std::thread::scope(|scope| {
        for standard_conforming_strings in [true, false] {
            scope.spawn(move || {
                for _ in 0..8 {
                    let options = ParseOptions {
                        mode: ParseMode::PlPgSqlExpr,
                        standard_conforming_strings,
                        escape_string_warning: false,
                        ..ParseOptions::default()
                    };
                    let expected = if standard_conforming_strings { r"a\nb" } else { "a\nb" };
                    assert_eq!(literal(r"'a\nb'", options), (expected.into(), vec![]));
                }
            });
        }
    });
}

#[test]
fn plpgsql_parser_uses_options_in_body_and_restores_them_after_error() {
    let sql = r"CREATE FUNCTION f() RETURNS text LANGUAGE plpgsql AS $$ BEGIN RETURN U&'d\0061t'; END $$";
    let outcome = pg_query::parse_plpgsql_with_options(sql, None, legacy());
    let Error::ParseDiagnostic(error) = outcome.result.unwrap_err() else { panic!("structured PL/pgSQL error") };
    assert_eq!(error.sqlstate, "0A000");
    assert_eq!(error.message, "unsafe use of string constant with Unicode escapes");
    assert!(pg_query::parse_plpgsql_with_options(sql, None, ParseOptions::default()).result.is_ok());
    assert_eq!(literal(r"SELECT 'a\nb'", ParseOptions::default()), (r"a\nb".into(), vec![]));
}

#[test]
fn scanner_preserves_legacy_quote_boundaries() {
    let sql = r"CREATE SERVER sample TYPE 'a\'b' VERSION '' FOREIGN DATA WRAPPER wrapper";
    let tokens = pg_query::scan_with_options(sql, ParseOptions { escape_string_warning: false, ..legacy() }).unwrap();
    assert_eq!(tokens.tokens.iter().filter(|token| token.token() == pg_query::protobuf::Token::Sconst).count(), 2);
    assert!(pg_query::scan_with_options(sql, ParseOptions::default()).is_err());
    assert_eq!(literal(r"SELECT 'a\nb'", ParseOptions::default()), (r"a\nb".into(), vec![]));
}

#[test]
fn structured_parser_options_work_with_upstream_apis() {
    let sql = r"SELECT 'a\'b' FROM contacts";
    let options = ParseOptions { escape_string_warning: false, ..legacy() };
    let parsed = pg_query::parse(sql, options).unwrap();
    assert_eq!(parsed.tables(), ["contacts"]);
    assert_eq!(pg_query::summary(sql, options, -1).unwrap().tables(), ["contacts"]);
    assert!(pg_query::fingerprint(sql, options, 0).is_ok());
    assert!(pg_query::parse(sql, 0).is_err());
}

#[test]
fn serialization_errors_keep_structured_diagnostics() {
    let sql = format!("SELECT 1{}", "%1".repeat(100_000));
    let outcome = parse_with_options(&sql, ParseOptions::default());
    let Error::ParseDiagnostic(diagnostic) = outcome.result.unwrap_err() else {
        panic!("structured serialization error");
    };
    assert_eq!(diagnostic.sqlstate, "54001");
    assert!(diagnostic.message.contains("stack depth limit exceeded"));
    assert_eq!(outcome.diagnostics.last(), Some(diagnostic.as_ref()));
    assert!(pg_query::parse("SELECT 1", 0).is_ok());
}
