use super::*;

fn params(pattern: &str) -> TabGrepParams {
    TabGrepParams {
        pattern: pattern.to_owned(),
        ignore_case: false,
        fixed_strings: false,
        max_matches: 100,
    }
}

#[test]
fn matcher_supports_regex_case_and_fixed_strings() {
    assert!(compile_matcher(&params("err(or)?"))
        .unwrap()
        .is_match("error"));

    let mut insensitive = params("needle");
    insensitive.ignore_case = true;
    assert!(compile_matcher(&insensitive).unwrap().is_match("NEEDLE"));

    let mut fixed = params("a+b");
    fixed.fixed_strings = true;
    assert!(compile_matcher(&fixed).unwrap().is_match("a+b"));
    assert!(!compile_matcher(&fixed).unwrap().is_match("aaab"));
}

#[test]
fn matcher_rejects_unbounded_or_invalid_requests() {
    let empty = compile_matcher(&params("")).unwrap_err();
    assert_eq!(empty.code, ErrorCode::InvalidParams);

    let invalid = compile_matcher(&params("[")).unwrap_err();
    assert_eq!(invalid.code, ErrorCode::InvalidParams);

    let mut too_many = params("ok");
    too_many.max_matches = MAX_MATCHES + 1;
    assert_eq!(
        compile_matcher(&too_many).unwrap_err().code,
        ErrorCode::InvalidParams
    );
}

#[test]
fn bounded_match_text_keeps_the_match_and_utf8_boundaries() {
    let line = format!("{}néedle{}", "x".repeat(5_000), "y".repeat(5_000));
    let found = line.find("néedle").unwrap();
    let (text, truncated) = bounded_match_text(&line, found, found + "néedle".len());
    assert!(truncated);
    assert!(text.contains("néedle"));
    assert!(text.starts_with('…'));
    assert!(text.ends_with('…'));
    assert!(text.len() <= MAX_MATCH_TEXT_BYTES + 6);
}
