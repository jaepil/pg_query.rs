#[macro_use]
mod support;

#[test]
fn it_can_parse_a_simple_function() {
    let result = pg_query::parse_plpgsql(
        "
        CREATE OR REPLACE FUNCTION cs_fmt_browser_version(v_name varchar, v_version varchar)
        RETURNS varchar AS $$
        BEGIN
            IF v_version IS NULL THEN
                RETURN v_name;
            END IF;
            RETURN v_name || '/' || v_version;
        END; \
        $$ LANGUAGE plpgsql;
        ",
    );
    assert!(result.is_ok());
    let result = result.unwrap();
    let expected = include_str!("data/plpgsql_simple.json");
    let actual = serde_json::to_string_pretty(&result).unwrap();
    pretty_assertions::assert_eq!(expected.trim(), actual.trim());
}

#[test]
fn it_can_parse_a_query_function() {
    let result = pg_query::parse_plpgsql(
        "
        CREATE OR REPLACE FUNCTION fn(input integer) RETURNS jsonb LANGUAGE plpgsql STABLE AS
        '
        DECLARE
            result jsonb;
        BEGIN
            SELECT details FROM t INTO result WHERE col = input;
            RETURN result;
        END;
        ';
        ",
    );
    assert!(result.is_ok());
    let result = result.unwrap();
    let expected = include_str!("data/plpgsql_query.json");
    let actual = serde_json::to_string_pretty(&result).unwrap();
    pretty_assertions::assert_eq!(expected.trim(), actual.trim());
}

#[test]
fn it_will_error_on_invalid_input() {
    let result = pg_query::parse_plpgsql("CREATE RANDOM ix_test ON contacts.person;");
    assert!(result.is_err());
    assert_eq!(result.err().unwrap(), pg_query::Error::Parse("syntax error at or near \"RANDOM\"".into()));
}

#[test]
fn it_preserves_quoted_plpgsql_type_name_identifiers() {
    let result = pg_query::parse_plpgsql(
        r#"
        CREATE FUNCTION quoted_type_names() RETURNS void AS $$
        DECLARE
            column_value "app.dot"."typed.dot"."id.dot"%TYPE;
            row_value "app.dot"."typed.dot"%ROWTYPE;
        BEGIN
            RETURN;
        END;
        $$ LANGUAGE plpgsql;
        "#,
    )
    .unwrap();

    let datums = result[0]["PLpgSQL_function"]["datums"].as_array().unwrap();
    let type_metadata =
        |refname: &str| &datums.iter().find(|datum| datum["PLpgSQL_var"]["refname"] == refname).unwrap()["PLpgSQL_var"]["datatype"]["PLpgSQL_type"];

    assert_eq!(type_metadata("column_value")["typname_identifiers"], serde_json::json!(["app.dot", "typed.dot", "id.dot"]));
    assert_eq!(type_metadata("row_value")["typname_identifiers"], serde_json::json!(["app.dot", "typed.dot"]));
}
