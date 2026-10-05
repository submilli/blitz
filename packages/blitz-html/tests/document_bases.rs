//! Fragment parsing keeps destination bases while staging an inert tree.
mod common;
use blitz_dom::{QualName, ns};
use common::{parse, q};
use style::properties::PropertyDeclaration;
use style::values::generics::image::GenericImage;

#[test]
fn parsed_secondary_document_inline_styles_use_the_destination_base() {
    let mut doc = parse("<base href='https://parent.test/current/'>");
    let child = doc.mutate().create_document_node();
    doc.mutate().document_metadata_mut(child).inherited_base_url =
        Some("https://child.test/captured/".parse().unwrap());
    let body = {
        let mut m = doc.mutate();
        let body = m.create_element(QualName::new(None, ns!(html), "body".into()), vec![]);
        m.pre_insert(body, child, None).unwrap();
        m.set_inner_html(body, "<p style='background-image:url(image.png)'></p>");
        body
    };
    let element = doc.get_node(body).unwrap().children[0];
    assert_ne!(element, q(&doc, "base"));
    let guard = doc.guard().read();
    let block = doc
        .get_node(element)
        .unwrap()
        .element_data()
        .unwrap()
        .style_attribute
        .as_ref()
        .unwrap()
        .read_with(&guard);
    let PropertyDeclaration::BackgroundImage(images) = &block.declarations()[0] else {
        panic!("background image declaration");
    };
    let GenericImage::Url(url) = &images.0[0] else {
        panic!("URL image");
    };
    assert_eq!(
        url.url().unwrap().as_str(),
        "https://child.test/captured/image.png"
    );
}
