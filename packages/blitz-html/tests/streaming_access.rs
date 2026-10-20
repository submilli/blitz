//! Streaming parse pauses through an embedder-owned, projected document cell.
use std::cell::RefCell;
use std::rc::{Rc, Weak};

use blitz_dom::{BaseDocument, DocumentConfig, DocumentMutator};
use blitz_html::{DocAccess, ParseStep, StreamingParser};

struct Owner {
    document: BaseDocument,
    mutations: usize,
}
struct Access(Weak<RefCell<Owner>>);
impl DocAccess for Access {
    fn with_mutator<R>(&self, f: impl FnOnce(&mut DocumentMutator<'_>) -> R) -> R {
        let owner = self.0.upgrade().expect("test retains the document owner");
        let mut owner = owner.borrow_mut();
        owner.mutations += 1;
        f(&mut owner.document.mutate())
    }
}

#[test]
fn projected_access_releases_borrows_at_scripts_and_resumes_the_same_document() {
    let owner = Rc::new(RefCell::new(Owner {
        document: BaseDocument::new(DocumentConfig::default()),
        mutations: 0,
    }));
    let mut parser = StreamingParser::with_access(Access(Rc::downgrade(&owner)), None);
    parser.feed("<body><div id=before>one</div><script>paused</script><p id=after>two</p>");
    parser.end_of_input();
    let ParseStep::Script(script) = parser.run() else {
        panic!("parser must pause at the inline script");
    };
    {
        let mut owner = owner
            .try_borrow_mut()
            .expect("script pause releases document");
        assert_eq!(owner.document.text_content_of(script).unwrap(), "paused");
        assert!(owner.document.query_selector("#after").unwrap().is_none());
        let before = owner.document.query_selector("#before").unwrap().unwrap();
        owner.document.mutate().set_text_content(before, "changed");
        assert!(owner.mutations > 0);
    }
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(parser.is_finished());
    let owner = owner.borrow();
    let before = owner.document.query_selector("#before").unwrap().unwrap();
    let after = owner.document.query_selector("#after").unwrap().unwrap();
    assert_eq!(owner.document.text_content_of(before).unwrap(), "changed");
    assert_eq!(owner.document.text_content_of(after).unwrap(), "two");
}

#[test]
fn custom_access_keeps_detached_document_parsing_inert() {
    let owner = Rc::new(RefCell::new(Owner {
        document: BaseDocument::new(DocumentConfig::default()),
        mutations: 0,
    }));
    let target = owner.borrow_mut().document.mutate().create_document_node();
    let mut parser = StreamingParser::with_access(Access(Rc::downgrade(&owner)), Some(target));
    parser.feed("<body><script>never active</script><p>done</p>");
    parser.end_of_input();
    let ParseStep::Script(script) = parser.run() else {
        panic!("parser reports the inert script token");
    };
    assert!(
        owner
            .borrow()
            .document
            .get_node(script)
            .unwrap()
            .element_data()
            .unwrap()
            .script_state
            .already_started
    );
    assert!(matches!(parser.run(), ParseStep::Done));
    assert!(parser.is_finished());
    assert_eq!(Rc::strong_count(&owner), 1);
    let owner = owner.borrow();
    let paragraph = owner
        .document
        .query_selector_in(target, "p")
        .unwrap()
        .unwrap();
    assert_eq!(owner.document.text_content_of(paragraph).unwrap(), "done");
}
