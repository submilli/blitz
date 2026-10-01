//! Programmatic focus eligibility is distinct from sequential tab navigation.
mod common;
use common::{parse, q};

#[test]
fn negative_tabindex_accepts_programmatic_focus_only() {
    let mut doc = parse(
        "<div id=a tabindex=-1></div><input id=b disabled><input id=c type=HIDDEN><fieldset disabled><input id=d></fieldset><input id=e>",
    );
    doc.resolve(0.0);
    let a = q(&doc, "#a");
    assert!(doc.get_node(a).unwrap().is_programmatically_focusable());
    assert!(!doc.get_node(a).unwrap().is_focussable());
    for name in ["#b", "#c", "#d"] {
        assert!(
            !doc.get_node(q(&doc, name))
                .unwrap()
                .is_programmatically_focusable()
        );
    }
    let e = q(&doc, "#e");
    assert!(doc.get_node(e).unwrap().is_programmatically_focusable());
    doc.mutate().remove_node(e);
    assert!(!doc.get_node(e).unwrap().is_programmatically_focusable());
}

#[test]
fn focus_rechecks_rendering_and_disabledness_after_mutation() {
    let mut doc = parse("<div id=p><input id=a></div><input id=b style='visibility:hidden'>");
    let (p, a, b) = (q(&doc, "#p"), q(&doc, "#a"), q(&doc, "#b"));
    doc.resolve(0.0);
    assert!(doc.get_node(a).unwrap().is_programmatically_focusable());
    assert!(!doc.get_node(b).unwrap().is_programmatically_focusable());
    doc.mutate()
        .set_attribute_by_name(p, "style", "display:none")
        .unwrap();
    doc.resolve(0.0);
    assert!(!doc.get_node(a).unwrap().is_programmatically_focusable());
    doc.mutate().remove_attribute_by_name(p, "style");
    doc.mutate()
        .set_attribute_by_name(a, "disabled", "")
        .unwrap();
    doc.resolve(0.0);
    assert!(!doc.get_node(a).unwrap().is_programmatically_focusable());
}

#[test]
fn contents_ancestors_and_html_integer_tabindex_are_focusable() {
    let mut doc = parse(
        "<div style='display:contents'><input id=a></div><div id=b tabindex=' -1'></div><div id=c tabindex='0junk'></div><div id=d tabindex=0 style='display:contents'></div>",
    );
    doc.resolve(0.0);
    for selector in ["#a", "#b", "#c"] {
        assert!(
            doc.get_node(q(&doc, selector))
                .unwrap()
                .is_programmatically_focusable(),
            "{selector}"
        );
    }
    assert!(!doc.get_node(q(&doc, "#b")).unwrap().is_focussable());
    assert!(doc.get_node(q(&doc, "#c")).unwrap().is_focussable());
    assert!(
        !doc.get_node(q(&doc, "#d"))
            .unwrap()
            .is_programmatically_focusable()
    );
}

#[test]
fn summary_focusability_changes_when_first_summary_is_moved() {
    let mut doc =
        parse("<details id=d open><summary id=a>A</summary><summary id=b>B</summary></details>");
    let (d, a, b) = (q(&doc, "#d"), q(&doc, "#a"), q(&doc, "#b"));
    doc.resolve(0.0);
    assert!(doc.get_node(a).unwrap().is_programmatically_focusable());
    assert!(!doc.get_node(b).unwrap().is_programmatically_focusable());
    doc.mutate().pre_insert(b, d, Some(a)).unwrap();
    doc.resolve(0.0);
    assert!(doc.get_node(b).unwrap().is_programmatically_focusable());
    assert!(!doc.get_node(a).unwrap().is_programmatically_focusable());
}

#[test]
fn batch_focus_preserves_candidate_order_and_matches_scalar_eligibility() {
    let mut doc = parse(
        "<fieldset disabled><legend><input id=a></legend><input id=b></fieldset><div inert><input id=c></div><div style='display:none'><input id=d></div><div style='display:contents'><input id=e></div><input id=f style='visibility:hidden'><input id=g>",
    );
    doc.resolve(0.0);
    let candidates: Vec<_> = ["#f", "#d", "#c", "#b", "#e", "#a", "#g"]
        .into_iter()
        .map(|s| q(&doc, s))
        .collect();
    for start in 0..candidates.len() {
        let candidates = &candidates[start..];
        let expected = candidates
            .iter()
            .copied()
            .find(|&id| doc.get_node(id).unwrap().is_programmatically_focusable());
        assert_eq!(doc.first_programmatically_focusable(candidates), expected);
    }
    let detached = q(&doc, "#e");
    doc.mutate().remove_node(detached);
    doc.resolve(0.0);
    assert_eq!(
        doc.first_programmatically_focusable(&candidates),
        Some(q(&doc, "#a"))
    );
}

#[test]
fn sequential_focus_batches_ancestor_checks_and_rechecks_mutations() {
    // Match the engine's deep-tree test stack; retain the full ancestry depth.
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(sequential_focus_case)
        .unwrap()
        .join()
        .unwrap();
}

fn sequential_focus_case() {
    let mut doc = parse(
        "<input id=start><div id=blocked inert></div><fieldset disabled><legend><input id=legend></legend><input id=disabled></fieldset><div style='display:none'><input></div><input id=last tabindex=-1>",
    );
    let blocked = q(&doc, "#blocked");
    let markup = format!(
        "{}<input id=deep>{}",
        "<div>".repeat(256),
        "</div>".repeat(256)
    );
    doc.mutate()
        .try_set_inner_html_detaching(blocked, &markup)
        .unwrap();
    doc.resolve(0.0);
    let start = q(&doc, "#start");
    let legend = q(&doc, "#legend");
    let deep = q(&doc, "#deep");
    doc.set_focus_to(start);
    assert_eq!(doc.focus_next_node(), Some(legend));
    assert_eq!(doc.focus_prev_node(), Some(start));
    doc.mutate().remove_attribute_by_name(blocked, "inert");
    doc.resolve(0.0);
    assert_eq!(doc.focus_next_node(), Some(deep));
    assert_eq!(doc.focus_next_node(), Some(legend));
    assert_eq!(doc.focus_prev_node(), Some(deep));
}

#[test]
fn native_tab_rechecks_focus_generation_after_blur_callback() {
    use blitz_dom::{Document, EventDriver, EventHandler, NodeId};
    use blitz_traits::events::{
        BlitzKeyEvent, DomEvent, DomEventData, EventState, KeyState, UiEvent,
    };
    struct Redirect {
        old: NodeId,
        next: NodeId,
        redirect: NodeId,
        seen: Vec<String>,
    }
    impl EventHandler for &mut Redirect {
        fn handle_event(
            &mut self,
            _: &[NodeId],
            event: &mut DomEvent,
            doc: &mut dyn Document,
            _: &mut EventState,
        ) {
            match &event.data {
                DomEventData::Blur(data) => {
                    assert_eq!(event.target, self.old);
                    assert_eq!(data.related_target, Some(self.next));
                    assert!(!doc.inner().get_node(self.old).unwrap().is_focussed());
                    assert!(!doc.inner().get_node(self.next).unwrap().is_focussed());
                    doc.inner_mut().set_focus_to(self.redirect);
                    self.seen.push("blur".into());
                }
                DomEventData::FocusOut(data) => {
                    assert_eq!(data.related_target, None);
                    self.seen.push("focusout".into());
                }
                DomEventData::Focus(_) | DomEventData::FocusIn(_) => {
                    panic!("superseded destination must receive no focus event")
                }
                _ => {}
            }
        }
    }
    let mut doc = parse("<input id=a><input id=b><input id=c>");
    doc.resolve(0.0);
    let (old, next, redirect) = (q(&doc, "#a"), q(&doc, "#b"), q(&doc, "#c"));
    doc.set_focus_to(old);
    let mut handler = Redirect {
        old,
        next,
        redirect,
        seen: vec![],
    };
    EventDriver::new(&mut doc, &mut handler).handle_ui_event(UiEvent::KeyDown(BlitzKeyEvent {
        key: keyboard_types::Key::Tab,
        code: keyboard_types::Code::Tab,
        modifiers: keyboard_types::Modifiers::empty(),
        location: keyboard_types::Location::Standard,
        is_auto_repeating: false,
        is_composing: false,
        state: KeyState::Pressed,
        text: None,
    }));
    assert_eq!(handler.seen, ["blur", "focusout"]);
    assert_eq!(doc.get_focussed_node_id(), Some(redirect));
}
