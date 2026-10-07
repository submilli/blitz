//! Capture actual DOM-generated requests, including CSS image and font paths.
use crate::{BaseDocument, DocumentConfig, qual_name};
use blitz_traits::net::{NetHandler, NetProvider, Request, ResourceInitiator};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct Capture(Mutex<Vec<(String, ResourceInitiator)>>);
impl NetProvider for Capture {
    fn fetch(&self, _: usize, request: Request, _: Box<dyn NetHandler>) {
        self.0
            .lock()
            .unwrap()
            .push((request.url.path().into(), request.initiator));
    }
}
#[test]
fn actual_dom_loads_retain_their_initiating_source() {
    let capture = Arc::new(Capture::default());
    let mut document = BaseDocument::new(DocumentConfig {
        base_url: Some("https://page.test/".into()),
        net_provider: Some(capture.clone()),
        ..Default::default()
    });
    let root = document.root_node().id;
    {
        let mut m = document.mutate();
        let html = m.create_element(qual_name!("html", html), vec![]);
        m.pre_insert(html, root, None).unwrap();
        let body = m.create_element(qual_name!("body", html), vec![]);
        m.pre_insert(body, html, None).unwrap();
        let image = m.create_element(qual_name!("img", html), vec![]);
        m.set_attribute(image, qual_name!("src"), "/image");
        m.pre_insert(image, body, None).unwrap();
        let input = m.create_element(qual_name!("input", html), vec![]);
        m.set_attribute(input, qual_name!("type"), "image");
        m.set_attribute(input, qual_name!("src"), "/input");
        m.pre_insert(input, body, None).unwrap();
        let link = m.create_element(qual_name!("link", html), vec![]);
        m.set_attribute(link, qual_name!("rel"), "stylesheet");
        m.set_attribute(link, qual_name!("href"), "/sheet");
        m.pre_insert(link, body, None).unwrap();
        let painted = m.create_element(qual_name!("div", html), vec![]);
        m.set_attribute(painted, qual_name!("id"), "paint");
        m.pre_insert(painted, body, None).unwrap();
        let style = m.create_element(qual_name!("style", html), vec![]);
        m.set_text_content(style, "#paint {display:block;width:10px;height:10px;background-image:url('/background');mask-image:url('/mask')} @font-face {font-family:Probe;src:url('/font')}");
        m.pre_insert(style, body, None).unwrap();
    }
    document.resolve(0.0);
    let requests = capture.0.lock().unwrap();
    for (path, source) in [
        ("/image", ResourceInitiator::Img),
        ("/input", ResourceInitiator::Input),
        ("/sheet", ResourceInitiator::Link),
        ("/background", ResourceInitiator::Css),
        ("/mask", ResourceInitiator::Css),
        ("/font", ResourceInitiator::Css),
    ] {
        assert!(
            requests
                .iter()
                .any(|(url, actual)| url == path && *actual == source),
            "missing {path} from {source:?}: {requests:?}"
        );
    }
}
