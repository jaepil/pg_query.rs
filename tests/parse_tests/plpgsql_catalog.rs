use pg_query::{parse_plpgsql, parse_plpgsql_with_catalog, PlpgsqlCatalog, PlpgsqlType};

fn catalog() -> PlpgsqlCatalog {
    PlpgsqlCatalog {
        namespaces: [("pg_catalog".into(), 11), ("app".into(), 900001)].into_iter().collect(),
        search_path: vec!["pg_catalog".into(), "app".into()],
        types: vec![PlpgsqlType {
            oid: 900002,
            namespace_oid: 900001,
            name: "positive".into(),
            length: 4,
            by_value: true,
            type_kind: b'd',
            category: b'N',
            alignment: b'i',
            storage: b'p',
            array_oid: 900003,
            element_oid: 0,
            base_type_oid: 23,
            collation_oid: 0,
            subscript_handler_oid: 0,
        }],
    }
}

#[test]
fn catalog_domain_declarations_keep_defaults_and_scalar_datums() {
    let parsed = parse_plpgsql_with_catalog("CREATE FUNCTION f(arg app.positive) RETURNS integer AS $$ DECLARE local_value CONSTANT positive NOT NULL := 7; BEGIN RETURN local_value; END $$ LANGUAGE plpgsql", &catalog()).unwrap();
    let datums = parsed[0]["PLpgSQL_function"]["datums"].as_array().unwrap();
    let local =
        datums.iter().filter_map(|datum| datum.get("PLpgSQL_var")).find(|datum| datum["refname"] == "local_value").expect("domain is a scalar datum");
    assert_eq!(local["datatype"]["PLpgSQL_type"]["typname"], "positive");
    assert_eq!(local["isconst"], true);
    assert_eq!(local["notnull"], true);
    assert!(local.get("default_val").is_some());
    assert!(datums.iter().any(|datum| datum["PLpgSQL_var"]["refname"] == "arg"));
}

#[test]
fn catalog_scope_does_not_escape_a_parse_or_thread() {
    let source = "CREATE FUNCTION f() RETURNS integer AS $$ DECLARE local_value positive; BEGIN RETURN local_value; END $$ LANGUAGE plpgsql";
    let standalone = parse_plpgsql(source);
    assert!(standalone.as_ref().unwrap().to_string().contains("PLpgSQL_rec"));
    for _ in 0..3 {
        let parsed = parse_plpgsql_with_catalog(source, &catalog()).unwrap();
        assert!(parsed.to_string().contains("PLpgSQL_var"));
        assert_eq!(parse_plpgsql(source), standalone);
    }
    let threads = (0..4).map(|_| std::thread::spawn(move || parse_plpgsql_with_catalog(source, &catalog()).unwrap())).collect::<Vec<_>>();
    for thread in threads {
        thread.join().unwrap();
    }
}

#[test]
fn malformed_catalog_metadata_is_an_error_and_does_not_poison_later_parses() {
    let source = "CREATE FUNCTION f(value app.positive) RETURNS integer AS $$ BEGIN RETURN 1; END $$ LANGUAGE plpgsql";
    let mut invalid = catalog();
    invalid.types[0].length = 0;
    assert!(parse_plpgsql_with_catalog(source, &invalid).unwrap_err().to_string().contains("invalid type metadata"));
    assert!(parse_plpgsql_with_catalog(source, &catalog()).is_ok());
    invalid.types[0].name = "positive\0invalid".into();
    assert!(parse_plpgsql_with_catalog(source, &invalid).is_err());
}
