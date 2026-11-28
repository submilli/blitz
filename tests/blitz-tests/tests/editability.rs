use blitz_test_harness::Harness;

#[test]
fn design_mode_changes_selector_matching_and_computed_style() {
    let mut page = Harness::from_html(
        "<style>#p:read-only {opacity: 0.25} #p:read-write {opacity: 0.75}</style><div id=p></div>",
    );
    let node = page.node("#p");
    let document = page.base().root_node().id;
    for enabled in [false, true, false] {
        page.base_mut().set_design_mode(document, enabled);
        page.pump();
        let doc = page.base();
        assert_eq!(doc.matches_selector(node, ":read-write").unwrap(), enabled);
        assert_eq!(doc.matches_selector(node, ":read-only").unwrap(), !enabled);
        assert_eq!(
            doc.resolved_style_value(node, "opacity"),
            if enabled { "0.75" } else { "0.25" }
        );
    }
}

#[test]
fn input_editors_and_contenteditable_have_distinct_rules() {
    let page = Harness::from_html(
        "<section contenteditable><input id=c type=checkbox><input id=r readonly><input id=t type=unknown><fieldset disabled><textarea id=d></textarea></fieldset><span id=s></span><i id=i contenteditable=false></i><svg id=v></svg></section>",
    );
    for (id, writable, readonly) in [
        ("#c", false, true),
        ("#r", false, true),
        ("#d", false, true),
        ("#t", true, false),
        ("#s", true, false),
        ("#i", false, true),
        ("#v", false, false),
    ] {
        let node = page.node(id);
        assert_eq!(
            page.base().matches_selector(node, ":read-write").unwrap(),
            writable,
            "{id}"
        );
        assert_eq!(
            page.base().matches_selector(node, ":read-only").unwrap(),
            readonly,
            "{id}"
        );
    }
}

#[test]
fn inherited_control_state_restyles_following_sibling() {
    let mut page = Harness::from_html(
        "<style>.child:read-write + span {opacity: 0.25}</style><div id=outer><main><section><fieldset id=f><input class=child><span id=marker></span></fieldset></section></main></div>",
    );
    let marker = page.node("#marker");
    let fieldset = page.node("#f");
    let disabled =
        blitz_dom::QualName::new(None, blitz_dom::ns!(), blitz_dom::local_name!("disabled"));
    for enabled in [true, false, true] {
        if enabled {
            page.base_mut()
                .mutate()
                .clear_attribute(fieldset, disabled.clone());
        } else {
            page.base_mut()
                .mutate()
                .set_attribute(fieldset, disabled.clone(), "");
        }
        page.pump();
        assert_eq!(
            page.base().resolved_style_value(marker, "opacity"),
            if enabled { "0.25" } else { "1" }
        );
    }
}

#[test]
fn changing_ancestor_keyword_refreshes_a_previously_styled_child() {
    let mut page = Harness::from_html(
        "<style>#child:read-write {opacity: 0.25}</style><main id=parent><b><span id=child></span></b></main>",
    );
    let parent = page.node("#parent");
    let child = page.node("#child");
    let keyword = blitz_dom::QualName::new(
        None,
        blitz_dom::ns!(),
        blitz_dom::local_name!("contenteditable"),
    );
    for (value, expected) in [
        (Some("plaintext-only"), true),
        (Some("false"), false),
        (Some("true"), true),
        (None, false),
    ] {
        match value {
            Some(value) => page
                .base_mut()
                .mutate()
                .set_attribute(parent, keyword.clone(), value),
            None => page
                .base_mut()
                .mutate()
                .clear_attribute(parent, keyword.clone()),
        }
        assert_eq!(
            page.base().matches_selector(child, ":read-write").unwrap(),
            expected
        );
        page.pump();
        assert_eq!(
            page.base().resolved_style_value(child, "opacity"),
            if expected { "0.25" } else { "1" }
        );
    }
}
