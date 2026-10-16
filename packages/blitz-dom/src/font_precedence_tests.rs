//! Host fonts stay available by generic role while author families shadow names.
use crate::net::{FontFaceOverrides, Resource, ResourceLoadResponse};
use crate::{BULLET_FONT, BaseDocument, DocumentConfig, FontRole, build_font_ctx};
use parley::fontique::{FontStyle, FontWeight, GenericFamily};

#[test]
fn page_family_shadows_host_faces_without_replacing_generic_fonts() {
    let mut fonts = build_font_ctx(&[(FontRole::SansSerif, BULLET_FONT)]);
    let host_id = fonts
        .collection
        .generic_families(GenericFamily::SansSerif)
        .next()
        .unwrap();
    let host_faces = fonts.collection.family(host_id).unwrap().fonts().len();
    let family_name = fonts.collection.family_name(host_id).unwrap().to_owned();
    let mut document = BaseDocument::new(DocumentConfig {
        font_ctx: Some(fonts),
        ..Default::default()
    });
    document.load_resource(ResourceLoadResponse {
        _node_pin: None,
        request_id: 0,
        node_id: None,
        resolved_url: Some("test://page-font".into()),
        result: Ok(Resource::Font(
            blitz_traits::net::Bytes::from_static(BULLET_FONT),
            FontFaceOverrides {
                family_name: Some(family_name.clone()),
                weight: Some(800.0),
                style: Some(FontStyle::Italic),
            },
        )),
    });
    let font_ctx = document.font_ctx();
    let mut fonts = font_ctx.lock().unwrap();
    let author_id = fonts.collection.family_id(&family_name).unwrap();
    assert_ne!(author_id, host_id);
    let author = fonts.collection.family(author_id).unwrap();
    assert_eq!(author.fonts().len(), 1);
    assert_eq!(author.fonts()[0].weight(), FontWeight::new(800.0));
    assert_eq!(author.fonts()[0].style(), FontStyle::Italic);
    assert_eq!(
        fonts
            .collection
            .generic_families(GenericFamily::SansSerif)
            .next(),
        Some(host_id)
    );
    assert_eq!(
        fonts.collection.family(host_id).unwrap().fonts().len(),
        host_faces
    );
}
