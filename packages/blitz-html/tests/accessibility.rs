//! The accessibility tree: names, states, values and bounds.

mod common;

use accesskit::{Invalid, Node, NodeId as AxId, Role, Toggled};
use blitz_dom::BaseDocument;
use common::{parse, q};

fn tree(doc: &mut BaseDocument) -> std::collections::HashMap<AxId, Node> {
    doc.resolve(0.0);
    doc.build_accessibility_tree().nodes.into_iter().collect()
}

fn node<'a>(
    tree: &'a std::collections::HashMap<AxId, Node>,
    doc: &BaseDocument,
    selector: &str,
) -> &'a Node {
    let id = q(doc, selector);
    tree.get(&AxId(id.as_u64()))
        .unwrap_or_else(|| panic!("{selector} is in the tree"))
}

#[test]
fn names_come_from_labels_attributes_and_content() {
    let mut doc = parse(
        "<label for=e>Email <b>address</b></label><input id=e required placeholder='you@example.com'>\
         <label>Remember me <input type=checkbox id=c checked></label>\
         <span id=l1>Billing</span><span id=l2>name</span>\
         <input id=b aria-labelledby='l1 l2' aria-label='ignored'>\
         <button id=btn aria-label='Close dialog'>×</button>\
         <a id=a href='/docs'>Read <img alt='the docs'></a>\
         <h2 id=h>Section <span style='display:none'>hidden</span> title</h2>\
         <img id=i alt='A cat'><input type=submit id=s><div id=t title='Tooltip only'></div>\
         <a id=up href='/vote'><div title=upvote></div></a>",
    );
    let tree = tree(&mut doc);
    let label = |sel| node(&tree, &doc, sel).label().map(str::to_string);
    assert_eq!(label("#e").as_deref(), Some("Email address"));
    assert_eq!(label("#c").as_deref(), Some("Remember me"));
    assert_eq!(label("#b").as_deref(), Some("Billing name"));
    assert_eq!(label("#btn").as_deref(), Some("Close dialog"));
    assert_eq!(label("#a").as_deref(), Some("Read the docs"));
    assert_eq!(label("#h").as_deref(), Some("Section title"));
    assert_eq!(label("#i").as_deref(), Some("A cat"));
    assert_eq!(label("#s").as_deref(), Some("Submit"));
    assert_eq!(label("#t").as_deref(), Some("Tooltip only"));
    // A descendant's tooltip names a link with no text.
    assert_eq!(label("#up").as_deref(), Some("upvote"));
}

#[test]
fn states_values_levels_and_urls() {
    let mut doc = parse(
        "<input id=e required placeholder=p value=typed><input type=checkbox id=c checked>\
         <button id=d disabled>Off</button><div role=button id=x aria-expanded=true aria-invalid=true>Menu</div>\
         <select id=s><option>One<option id=o2 selected>Two</select>\
         <details open><summary id=sum>More</summary>body</details>\
         <h3 id=h>Level</h3><div role=heading aria-level=5 id=h5>Deep</div><a id=a href='/next?x=1'>next</a>\
         <input id=r readonly aria-describedby=help><p id=help>Your id</p>",
    );
    let tree = tree(&mut doc);
    let n = |sel| node(&tree, &doc, sel);
    assert!(n("#e").is_required());
    assert_eq!(n("#e").value(), Some("typed"));
    assert_eq!(n("#e").placeholder(), Some("p"));
    assert_eq!(n("#c").toggled(), Some(Toggled::True));
    assert!(n("#d").is_disabled());
    assert_eq!(n("#x").is_expanded(), Some(true));
    assert_eq!(n("#x").invalid(), Some(Invalid::True));
    assert_eq!(n("#s").value(), Some("Two"));
    assert_eq!(n("#o2").is_selected(), Some(true));
    // Options are not rendered while collapsed, but keep their text as name.
    assert_eq!(n("#o2").label(), Some("Two"));
    assert_eq!(n("#sum").is_expanded(), Some(true));
    assert_eq!(n("#h").level(), Some(3));
    assert_eq!(n("#h5").level(), Some(5));
    assert_eq!(n("#a").url(), Some("https://example.test/next?x=1"));
    assert!(n("#r").is_read_only());
    assert_eq!(n("#r").description(), Some("Your id"));
}

#[test]
fn hidden_elements_are_left_out_and_bounds_come_from_layout() {
    let mut doc = parse(
        "<body style='margin:0'><div id=box role=region aria-label=Box style='margin-left:10px;width:100px;height:40px'></div>\
         <div style='display:none'><button id=gone>Hidden</button></div></body>",
    );
    let tree = tree(&mut doc);
    let bounds = node(&tree, &doc, "#box").bounds().expect("bounds");
    assert_eq!(
        (bounds.x0, bounds.y0, bounds.x1, bounds.y1),
        (10.0, 0.0, 110.0, 40.0)
    );
    assert!(!tree.contains_key(&AxId(q(&doc, "#gone").as_u64())));
    assert_eq!(node(&tree, &doc, "#box").role(), Role::Region);
}
