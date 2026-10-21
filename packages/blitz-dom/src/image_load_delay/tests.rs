use crate::image_state::ImageUpdateStage;
use crate::{BaseDocument, DocumentConfig, NodeId, qual_name};
use blitz_traits::net::{NetHandler, NetProvider, Request, ResourceInitiator};
use std::sync::{Arc, Mutex};
#[derive(Default)]
struct Provider(Mutex<Vec<(Request, Box<dyn NetHandler>)>>);
impl NetProvider for Provider {
    fn fetch(&self, _: usize, request: Request, handler: Box<dyn NetHandler>) {
        self.0.lock().unwrap().push((request, handler));
    }
}
impl Provider {
    fn fail(&self, document: &mut BaseDocument) {
        let (_, handler) = self.0.lock().unwrap().remove(0);
        handler.failed();
        document.handle_messages();
    }
}
fn document(provider: Arc<Provider>) -> (BaseDocument, NodeId) {
    let mut document = BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/".into()),
        net_provider: Some(provider),
        ..Default::default()
    });
    document.embedder_updates_images = true;
    let root = document.root_node().id;
    let mut m = document.mutate();
    let html = m.create_element(qual_name!("html", html), vec![]);
    m.append_children(root, &[html]);
    let body = m.create_element(qual_name!("body", html), vec![]);
    m.append_children(html, &[body]);
    drop(m);
    (document, body)
}
fn image(document: &mut BaseDocument, parent: NodeId, input: bool) -> NodeId {
    let mut m = document.mutate();
    let node = m.create_element(
        if input {
            qual_name!("input", html)
        } else {
            qual_name!("img", html)
        },
        vec![],
    );
    if input {
        m.set_attribute(node, qual_name!("type"), "image");
    }
    m.set_attribute(node, qual_name!("src"), "https://page.test/held.png");
    m.append_children(parent, &[node]);
    node
}
fn stable(document: &mut BaseDocument) {
    let root = document.root_node().id;
    document.run_image_updates(ImageUpdateStage::StableState, |document| document == root);
}
#[test]
fn removed_image_waiter_releases_load_delay_before_transport_finishes() {
    let provider = Arc::new(Provider::default());
    let (mut doc, body) = document(provider.clone());
    let root = doc.root_node().id;
    let node = image(&mut doc, body, false);
    stable(&mut doc);
    assert_eq!(provider.0.lock().unwrap().len(), 1);
    assert!(doc.image_resources_delay_load(root));
    doc.mutate().remove_attribute_by_name(node, "src");
    stable(&mut doc);
    assert!(!doc.image_resources_delay_load(root));
    assert_eq!(
        provider.0.lock().unwrap().len(),
        1,
        "transport remains independently owned"
    );
    provider.fail(&mut doc);
    assert!(!doc.image_resources_delay_load(root));
}
#[test]
fn image_inputs_delay_load_until_response_or_source_removal() {
    for removed in [false, true] {
        let provider = Arc::new(Provider::default());
        let (mut doc, body) = document(provider.clone());
        let root = doc.root_node().id;
        let node = image(&mut doc, body, true);
        stable(&mut doc);
        assert_eq!(provider.0.lock().unwrap().len(), 1);
        assert!(
            !doc.images_delay_load(root),
            "legacy query is HTML img-specific"
        );
        assert!(doc.image_resources_delay_load(root));
        if removed {
            doc.mutate().remove_attribute_by_name(node, "src");
            stable(&mut doc);
            assert!(!doc.image_resources_delay_load(root));
        }
        provider.fail(&mut doc);
        assert!(!doc.image_resources_delay_load(root));
    }
}
#[test]
fn css_waiter_keeps_shared_fetch_load_delay_after_image_source_removal() {
    for css_first in [false, true] {
        let provider = Arc::new(Provider::default());
        let (mut doc, body) = document(provider.clone());
        let root = doc.root_node().id;
        doc.mutate().set_attribute(
            body,
            qual_name!("style"),
            "width:32px;height:32px;background-image:url('https://page.test/held.png')",
        );
        if css_first {
            doc.resolve(0.0);
        }
        let node = image(&mut doc, body, false);
        stable(&mut doc);
        doc.resolve(0.0);
        let requests = provider.0.lock().unwrap();
        assert_eq!(requests.len(), 1, "one shared fetch");
        assert_eq!(
            requests[0].0.initiator,
            if css_first {
                ResourceInitiator::Css
            } else {
                ResourceInitiator::Img
            }
        );
        drop(requests);
        doc.mutate().remove_attribute_by_name(node, "src");
        stable(&mut doc);
        assert!(!doc.images_delay_load(root));
        assert!(
            doc.image_resources_delay_load(root),
            "background still needs the shared response"
        );
        provider.fail(&mut doc);
        assert!(!doc.image_resources_delay_load(root));
    }
}
