use regress_bounded::{Budget, Pattern, PatternError};

fn utf16(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn matches(pattern: &str, value: &str) -> bool {
    let budget = Budget::default();
    Pattern::compile(&utf16(pattern), &budget)
        .unwrap_or_else(|e| panic!("{pattern:?}: {e:?}"))
        .is_full_match(&utf16(value), &budget)
        .unwrap()
}

#[test]
fn full_match_keeps_longer_alternatives_and_lookaround_scope() {
    for (pattern, value, expected) in [
        ("a|ab", "ab", true),
        ("a|ab", "abc", false),
        ("(?=ab)a.*", "ab", true),
        ("a(?<=a)b", "ab", true),
        ("(?!b)a", "a", true),
        (r"(ab)\1", "abab", true),
        (r"(?<x>ab)\k<x>", "abab", true),
        ("", "", true),
        ("a$", "a\n", false),
    ] {
        assert_eq!(matches(pattern, value), expected, "{pattern:?}, {value:?}");
    }
}

#[test]
fn unicode_sets_strings_case_modifiers_and_surrogates() {
    for (pattern, value, expected) in [
        (r"[\p{ASCII}&&\p{Letter}]+", "Az", true),
        (r"[\p{ASCII}&&\p{Letter}]+", "é", false),
        (r"[[a-z]--[aeiou]]+", "bc", true),
        (r"[[a-z]--[aeiou]]+", "ab", false),
        (r"[\q{ab|a}]", "ab", true),
        (r"\p{RGI_Emoji}", "😀", true),
        ("(?i:ab)", "AB", true),
        ("(?i:[a-z])", "B", true),
        (".", "😀", true),
    ] {
        assert_eq!(matches(pattern, value), expected, "{pattern:?}, {value:?}");
    }
    let budget = Budget::default();
    let pattern = Pattern::compile(&[0xd800], &budget).unwrap();
    assert!(pattern.is_full_match(&[0xd800], &budget).unwrap());
    assert!(!pattern.is_full_match(&[0xfffd], &budget).unwrap());
}

#[test]
fn syntax_and_resource_errors_are_distinct() {
    assert!(matches!(
        Pattern::compile(&utf16("["), &Budget::default()),
        Err(PatternError::Syntax)
    ));
    for budget in [Budget::new(0, 10000), Budget::new(10000, 0)] {
        assert!(matches!(
            Pattern::compile(&utf16("a"), &budget),
            Err(PatternError::Resource(_))
        ));
        assert!(budget.check().is_err());
    }
    let source = format!("{}a{}", "(".repeat(33), ")".repeat(33));
    assert!(matches!(
        Pattern::compile(&utf16(&source), &Budget::default()),
        Err(PatternError::Resource(_))
    ));
}

#[test]
fn matching_exhaustion_is_sticky_and_shared() {
    let pattern = Pattern::compile(&utf16("a|ab"), &Budget::default()).unwrap();
    let budget = Budget::new(1, 10000);
    assert!(pattern.is_full_match(&utf16("ab"), &budget).is_err());
    assert!(pattern.is_full_match(&utf16("a"), &budget.clone()).is_err());
    assert!(
        pattern
            .is_full_match(&utf16("ab"), &Budget::default())
            .unwrap()
    );
}

#[test]
fn expanded_unicode_data_is_admitted_before_compilation() {
    let source = utf16(r"\p{RGI_Emoji}");
    // Enough for source-derived nodes, insufficient for expanded Unicode data.
    let budget = Budget::new(4_000_000, source.len() * 2048 + 128);
    assert!(matches!(
        Pattern::compile(&source, &budget),
        Err(PatternError::Resource(_))
    ));
}

#[test]
fn unicode_v_class_algebra_and_strings_preserve_browser_semantics() {
    for (pattern, value, expected) in [
        ("[a&]", "a", true),
        ("[a&]", "&", true),
        (r"a[\q{|b}]", "a", true),
        (r"[\b]", "\u{8}", true),
        (r"[\b]", "b", false),
        (r"[\q{\b}]", "\u{8}", true),
        ("[!#]", "!", true),
        ("(?i:[a--A])", "a", false),
        ("(?i:[a&&A])", "A", true),
        (r"(?i:[\q{AB}&&\q{ab}])", "aB", true),
        (r"(?i:\P{Lowercase_Letter})", "A", false),
        (r"(?i:[\P{Lowercase_Letter}])", "A", false),
        (r"(?i:[\W])", "K", false),
        (r"[^\q{a}]", "b", true),
        (r"[^\q{a}]", "a", false),
        (r"[^\q{ab}&&a]", "b", true),
        (r"ab(?<=[\q{ab}])", "ab", true),
        (r"(?:(?<x>a)|(?<x>b))\k<x>", "bb", true),
        (r"(?:(?<x>a)|(?<x>b))\k<x>", "b", false),
    ] {
        assert_eq!(matches(pattern, value), expected, "{pattern:?}, {value:?}");
    }
    for pattern in [r"[^\q{ab}]", r"[^\q{}]", r"[^\q{ab}--\q{ab}]", "[!!]"] {
        assert!(
            matches!(
                Pattern::compile(&utf16(pattern), &Budget::default()),
                Err(PatternError::Syntax)
            ),
            "{pattern}"
        );
    }
}

#[test]
fn small_budgets_never_turn_failure_into_a_panic_or_success() {
    let source = utf16("(?<x>a|ab)c");
    let pattern = Pattern::compile(&source, &Budget::default()).unwrap();
    for work in 0..800 {
        let _ = Pattern::compile(&source, &Budget::new(work, 100_000));
        let result = pattern.is_full_match(&utf16("abd"), &Budget::new(work, 100_000));
        assert!(!matches!(result, Ok(true)));
    }
}

#[test]
fn class_ordering_ranges_and_reserved_tokens() {
    for (pattern, value, expected) in [
        (r"(?=([\q{|a}]))\1", "a", true),
        ("[a&-z]", "B", true),
        ("[a&-z]", "a", true),
    ] {
        assert_eq!(matches(pattern, value), expected);
    }
    for pattern in [r"[\q{]}]", r"[\q{[}]", "[a&&&]", "[a-b&&c]"] {
        assert!(
            matches!(
                Pattern::compile(&utf16(pattern), &Budget::default()),
                Err(PatternError::Syntax)
            ),
            "{pattern}"
        );
    }
    for source in ["[a&&a]", "[a--b]", "[a&-z]", r"[\q{a|ab}]"] {
        for work in 0..1500 {
            let _ = Pattern::compile(&utf16(source), &Budget::new(work, 100_000));
        }
    }
}

#[test]
fn numeric_admission_has_the_same_bound_on_native_and_wasm() {
    for source in [
        "a{4294967296}",
        "a{99999999999999999999,88888888888888888888}",
    ] {
        assert!(matches!(
            Pattern::compile(&utf16(source), &Budget::default()),
            Err(PatternError::Resource(_))
        ));
    }
    assert!(matches!(
        Pattern::compile(&utf16("a{4,3}"), &Budget::default()),
        Err(PatternError::Syntax)
    ));
}
