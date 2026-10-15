//! What SVG documents expand to: refused or reserved before usvg allocates
//! it, through every entry point, while ordinary documents still parse.

use super::{MAX_NESTING, conversion, expand_entities};
use crate::DecodedImageBudget;
use crate::node::SvgImageData;
use crate::svg_bounds::{SvgRefusal, parse_svg_image};
use crate::util::default_svg_fonts;
use usvg::roxmltree;

mod allocations;
mod work;

const OPEN: &str = "<svg xmlns='http://www.w3.org/2000/svg' width='100' height='100'>";

fn parse(source: &str) -> Result<SvgImageData, SvgRefusal> {
    parse_svg_image(
        source.as_bytes(),
        &default_svg_fonts(),
        &DecodedImageBudget::default(),
    )
}

fn svg(body: &str) -> String {
    format!("{OPEN}{body}</svg>")
}

/// What the measure reserves to convert `source`.
fn reserved(source: &str) -> Option<usize> {
    let doc = crate::svg_root_size::parse_xml(source).unwrap();
    conversion(&doc).map(|conversion| conversion.bytes)
}

/// `data` as a percent-encoded `data:` SVG in an `<image>`.
fn nesting(data: &str) -> String {
    let href: String = data.bytes().map(|byte| format!("%{byte:02X}")).collect();
    format!("<image width='4' height='4' href='data:image/svg+xml,{href}'/>")
}

/// Run `test` on a thread with the guest's stack (`-zstack-size`), which
/// a debug build needs for the deepest documents admitted.
fn on_guest_stack(test: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(8 << 20)
        .spawn(test)
        .unwrap()
        .join()
        .unwrap();
}

fn images(svg: &SvgImageData) -> usize {
    let mut count = 0;
    crate::svg_walk::walk(&svg.tree, |visit| {
        if let crate::svg_walk::Visit::Node(usvg::Node::Image(_)) = visit {
            count += 1;
        }
    });
    count
}

/// `levels` entities, each referring ten times to the one before: at seven,
/// 500 bytes that roxmltree would expand to a million elements.
fn entity_bomb(levels: usize) -> String {
    let mut dtd = String::from("<!DOCTYPE svg [<!ENTITY e0 '<rect width=\"1\" height=\"1\"/>'>");
    for level in 1..levels {
        let previous = format!("&e{};", level - 1).repeat(10);
        dtd.push_str(&format!("<!ENTITY e{level} '{previous}'>"));
    }
    dtd.push(']');
    dtd.push('>');
    format!("<?xml version='1.0'?>{dtd}{OPEN}&e{};</svg>", levels - 1)
}

#[test]
fn entities_amplifying_a_document_are_refused_before_parsing() {
    assert!(expand_entities(&entity_bomb(7)).is_none());
    assert!(matches!(parse(&entity_bomb(7)), Err(SvgRefusal::Expansion)));
    // Two levels expand within libxml2's allowance, and are reserved.
    let source = entity_bomb(2);
    let small = expand_entities(&source).unwrap();
    let literal = source.matches('<').count();
    assert_eq!(small.tags, literal + 10, "{small:?}");
    assert!(parse(&entity_bomb(2)).is_ok());
}

/// Quotes in the comments and processing instructions of a DTD open
/// nothing, and a scan that cannot find where markup ends refuses.
#[test]
fn quotes_in_dtd_comments_hide_no_references() {
    let bomb = |aside: &str| {
        format!(
            "<!DOCTYPE svg [{aside}<!ENTITY a '{}'><!ENTITY b '{}'>]>{}",
            "x".repeat(1000),
            "&a;".repeat(250),
            svg(&format!("<text>{}</text>", "&b;".repeat(1000)))
        )
    };
    for aside in ["", "<!-- it's -->", "<?pi it's ?>", "<!-- \" -->"] {
        assert!(expand_entities(&bomb(aside)).is_none(), "{aside}");
    }
    assert!(expand_entities(&format!("{OPEN}<!-- unclosed")).is_none());
    assert!(expand_entities("<!DOCTYPE svg [<!ENTITY a 'x'>").is_none());
}

#[test]
fn non_ascii_markup_is_scanned_without_panicking() {
    let source = format!(
        "<!DOCTYPE svg [<!ENTITY é 'x'> <!-- ü --> ]>{}",
        svg("<text>&é; ü</text>")
    );
    assert!(expand_entities(&source).is_some());
    assert!(parse(&source).is_ok());
}

#[test]
fn entities_expanding_text_and_attributes_are_bounded() {
    let text = format!(
        "<!DOCTYPE svg [<!ENTITY a '{}'><!ENTITY b '&a;&a;&a;&a;&a;&a;&a;&a;'>]>{OPEN}<text>{}</text></svg>",
        "x".repeat(1000),
        "&b;".repeat(1000)
    );
    assert!(expand_entities(&text).is_none(), "8 MB of text");
    let attribute = format!(
        "<!DOCTYPE svg [<!ENTITY a '{}'>]>{OPEN}<path d='&a;{}'/></svg>",
        "L1 1 ".repeat(20_000),
        "&a;".repeat(100)
    );
    assert!(expand_entities(&attribute).is_none(), "10 MB of path data");
}

/// Illustrator exports declare namespaces as entities and refer to each a
/// few times.
#[test]
fn ordinary_entities_parse_as_before() {
    let source = "<?xml version='1.0'?>\
        <!DOCTYPE svg PUBLIC '-//W3C//DTD SVG 1.1//EN' 'http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd' [\
        <!ENTITY ns_svg 'http://www.w3.org/2000/svg'>\
        <!ENTITY ns_xlink 'http://www.w3.org/1999/xlink'>\
        <!ENTITY st0 'fill:#2D2D2D;'>]>\
        <svg xmlns='&ns_svg;' xmlns:xlink='&ns_xlink;' width='10' height='10'>\
        <rect style='&st0;' width='10' height='10'/></svg>";
    let svg = parse(source).unwrap();
    assert_eq!(svg.tree.root().children().len(), 1);
}

#[test]
fn use_copies_are_counted_for_each_copy() {
    // Five levels of ten `<use>` of the level below: 100,000 rectangles.
    let mut body = String::from("<defs><rect id='l0' width='1' height='1'/>");
    for level in 1..=5 {
        body.push_str(&format!("<g id='l{level}'>"));
        for _ in 0..10 {
            body.push_str(&format!("<use href='#l{}'/>", level - 1));
        }
        body.push_str("</g>");
    }
    body.push_str("</defs><use href='#l5'/>");
    let source = svg(&body);
    assert!(reserved(&source).unwrap() > 100_000 * 1024);
    assert!(matches!(parse(&source), Err(SvgRefusal::Budget)));
}

/// usvg copies a `<use>` target wherever it stands, also where it builds
/// nothing itself.
#[test]
fn use_targets_usvg_builds_only_as_copies_are_counted() {
    for (open, close) in [
        ("<unknown>", "</unknown>"),
        ("<text>", "</text>"),
        ("<use>", "</use>"),
    ] {
        let mut body = String::from("<rect id='l0' width='1' height='1'/>");
        for level in 1..=4 {
            let uses = format!("<use href='#l{}'/>", level - 1).repeat(10);
            body.push_str(&format!("{open}<g id='l{level}'>{uses}</g>{close}"));
        }
        let bytes = reserved(&svg(&format!("{body}<use href='#l4'/>"))).unwrap();
        assert!(bytes > 10_000 * 1024, "{open}: {bytes}");
    }
}

#[test]
fn text_is_counted_by_its_characters() {
    let large = svg(&format!("<text y='10'>{}</text>", "x".repeat(200_000)));
    assert!(reserved(&large).unwrap() > 200_000 * 1024);
    assert!(matches!(parse(&large), Err(SvgRefusal::Budget)));
    let copied = svg(&format!(
        "<text id='t'>{}</text>{}",
        "x".repeat(1000),
        "<text><tref href='#t'/></text>".repeat(200)
    ));
    assert!(reserved(&copied).unwrap() > 200 * 1000 * 1024);
    assert!(parse(&svg(&format!("<text y='10'>{}</text>", "x".repeat(2000)))).is_ok());
}

#[test]
fn arcs_are_counted_by_their_curves() {
    for shape in [
        "<circle r='3e38'/>",
        "<ellipse rx='1' ry='3e38'/>",
        "<rect width='1e38' height='1e38' rx='1e38'/>",
        "<path d='M0 0A1e38 1e38 0 1 1 2e38 0'/>",
        "<path d='M0 0A1 1 0 1 1 1e300 0'/>",
        "<g font-size='1e30'><circle r='1e8em'/></g>",
        "<svg width='1e30' height='1e30'><circle r='1e8%'/></svg>",
    ] {
        assert!(
            matches!(parse(&svg(shape)), Err(SvgRefusal::Budget)),
            "{shape}"
        );
    }
    assert!(
        parse(&svg(
            "<circle r='50%'/><circle r='2em'/><rect width='10' height='10' rx='2'/>"
        ))
        .is_ok()
    );
}

#[test]
fn markers_are_counted_for_each_vertex() {
    let points: String = (0..2000).map(|i| format!(" L{i} 0")).collect();
    let markers = format!(
        "<defs><marker id='a'><path d='M0 0{points}'/></marker>\
         <marker id='b'><path d='M0 0{points}' marker-mid='url(#a)'/></marker></defs>\
         <path d='M0 0{points}' marker-mid='url(#b)'/>"
    );
    // Each of 2,000 vertices draws a marker with 2,000 markers of 2,000 curves.
    assert!(reserved(&svg(&markers)).unwrap() > 2000 * 2000 * 2000);
    // Inherited from a group, counted on each path.
    let inherited = format!(
        "<defs><marker id='a'><path d='M0 0{points}'/></marker></defs>\
         <g marker-mid='url(#a)'>{}</g>",
        format!("<path d='M0 0{points}'/>").repeat(10)
    );
    assert!(reserved(&svg(&inherited)).unwrap() > 10 * 2000 * 2000 * 64);
}

#[test]
fn paint_servers_are_counted_for_each_painted_element() {
    let stops = "<stop offset='0'/>".repeat(800);
    let painted = "<rect width='1' height='1'/>".repeat(6400);
    let source = svg(&format!(
        "<linearGradient id='g'>{stops}</linearGradient><g fill='url(#g)'>{painted}</g>"
    ));
    assert!(reserved(&source).unwrap() > 6400 * 800 * 64);
    let css = svg(&format!(
        "<style>rect {{ fill: url(#g) }}</style><linearGradient id='g'>{stops}</linearGradient>{painted}"
    ));
    assert!(reserved(&css).unwrap() > 6400 * 800 * 64);
}

/// usvg copies an inherited dash list and font family into each element it
/// paints, makes a filter of each filter function, and clones paint servers
/// for each chunk of text, which may be a character.
#[test]
fn copies_per_painted_element_are_counted() {
    let paths = "<path d='M0 0h1'/>".repeat(5000);
    let dashes = svg(&format!(
        "<g stroke='black' stroke-dasharray='{}'>{paths}</g>",
        "1 ".repeat(50_000)
    ));
    assert!(reserved(&dashes).unwrap() > 5000 * 100_000 * 8);
    let families = svg(&format!(
        "<g font-family='{}'>{}</g>",
        "a,".repeat(10_000),
        "<text>x</text>".repeat(500)
    ));
    assert!(reserved(&families).unwrap() > 500 * 20_000 * 8);
    let functions = svg(&format!(
        "<rect id='r' width='1' height='1' filter='{}'/>{}",
        "blur(1) ".repeat(2000),
        "<use href='#r'/>".repeat(20)
    ));
    assert!(reserved(&functions).unwrap() > 20 * 2000 * 1024);
    let positions: String = (0..2000).map(|x| format!("{x} ")).collect();
    let text = svg(&format!(
        "<pattern id='p' width='1' height='1'>{}</pattern><text x='{positions}' fill='url(#p)'>{}</text>",
        "<rect width='1' height='1'/>".repeat(500),
        "x".repeat(2000)
    ));
    assert!(reserved(&text).unwrap() > 2000 * 500 * 1024);
}

#[test]
fn copied_declarations_are_counted_for_each_element() {
    let long = "a".repeat(10_000);
    let source = svg(&format!(
        "<style>rect {{ font-family: {long} }}</style>{}",
        "<rect width='1' height='1'/>".repeat(1000)
    ));
    assert!(reserved(&source).unwrap() > 1000 * 10_000);
}

/// Ordinary text painted with a gradient is not refused: usvg clones the
/// paint per chunk and span, not per character.
#[test]
fn ordinary_painted_text_is_admitted() {
    let spans = format!("<tspan>{}</tspan>", "Some words of text. ".repeat(6)).repeat(200);
    let source = svg(&format!(
        "<linearGradient id='g'><stop offset='0'/><stop offset='1' stop-color='red'/></linearGradient>\
         <text y='20' fill='url(#g)'>{spans}</text>"
    ));
    assert!(parse(&source).is_ok());
}

/// The measure keeps what each element declares; rules matching every
/// element cannot make it hold more than it reserves.
#[test]
fn declarations_kept_by_the_measure_are_bounded() {
    let rules: String = (0..200)
        .map(|i| format!("rect {{ fill: url(#p{i}) }}"))
        .collect();
    let source = svg(&format!(
        "<style>{rules}</style>{}",
        "<rect width='1' height='1'/>".repeat(1000)
    ));
    assert!(reserved(&source).is_none());
    assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
}

#[test]
fn nesting_past_what_usvg_recurses_is_refused() {
    let chain = |links: usize| {
        let mut body = String::from("<defs><rect id='u0' width='1' height='1'/>");
        for link in 1..links {
            body.push_str(&format!("<use id='u{link}' href='#u{}'/>", link - 1));
        }
        body.push_str(&format!("</defs><use href='#u{}'/>", links - 1));
        svg(&body)
    };
    assert!(matches!(parse(&chain(200)), Err(SvgRefusal::Expansion)));
    let groups = |depth: usize| svg(&format!("{}{}", "<g>".repeat(depth), "</g>".repeat(depth)));
    on_guest_stack(move || assert!(matches!(parse(&groups(200)), Err(SvgRefusal::Expansion))));

    // What is admitted converts within the guest's stack, in a debug build.
    on_guest_stack(move || assert!(parse(&chain(MAX_NESTING - 8)).is_ok()));
}

/// roxmltree recurses once per element level, known to usvg or not, so
/// deeper sources are refused before it parses them, on any stack.
#[test]
fn sources_nesting_past_what_the_parser_recurses_are_refused() {
    let unknown = |depth: usize| svg(&format!("{}{}", "<x>".repeat(depth), "</x>".repeat(depth)));
    assert!(matches!(
        parse(&unknown(100_000)),
        Err(SvgRefusal::Expansion)
    ));
    let entities = |inner: &str| {
        format!(
            "<!DOCTYPE svg [<!ENTITY a '{}{}'><!ENTITY b '{inner}'>]>{}",
            "<x>".repeat(150),
            "</x>".repeat(150),
            svg("<x>&b;</x>")
        )
    };
    assert!(
        expand_entities(&entities("&a;&a;")).is_some(),
        "side by side"
    );
    assert!(
        expand_entities(&entities("<x>&a;</x><x>&a;</x>")).is_some(),
        "side by side in elements"
    );
    let nested = entities(&format!("{}&a;{}", "<x>".repeat(150), "</x>".repeat(150)));
    assert!(
        expand_entities(&nested).is_none(),
        "nested through entities"
    );
    // Markup that holds no elements does not nest.
    let inert = svg(&format!(
        "<!--{0}--><![CDATA[{0}]]><?pi {0}?><rect title='{1}' width='1' height='1'/>",
        "<g>".repeat(1000),
        "&gt;".repeat(10)
    ));
    assert!(parse(&inert).is_ok());
}

#[test]
fn reference_cycles_are_refused_and_self_references_are_not() {
    let cycle = svg(
        "<pattern id='a' width='1' height='1'><rect width='1' height='1' fill='url(#b)'/></pattern>\
         <pattern id='b' width='1' height='1'><rect width='1' height='1' fill='url(#a)'/></pattern>\
         <rect width='1' height='1' fill='url(#a)'/>",
    );
    assert!(matches!(parse(&cycle), Err(SvgRefusal::Expansion)));
    // usvg skips a `<use>` inside its own target, and a marker inside its own
    // content; a stylesheet marking every path marks the marker's too.
    let skipped = svg("<g id='g'><rect width='1' height='1'/><use href='#g'/></g>\
         <style>path { marker-end: url(#m) }</style>\
         <marker id='m'><path d='M0 0L1 1'/></marker><path d='M0 0L5 5'/>");
    assert!(parse(&skipped).is_ok());
}

/// A nested SVG image is bounded like the image nesting it, and the
/// reservations of both are held until the image is parsed.
#[test]
fn nested_documents_are_bounded_by_their_expansion() {
    let bomb = nesting(&entity_bomb(7));
    let svg_with_bomb = parse(&svg(&bomb)).unwrap();
    assert_eq!(images(&svg_with_bomb), 0, "the nested bomb is refused");

    let text = nesting(&svg(&format!("<text>{}</text>", "x".repeat(20_000))));
    let budget = DecodedImageBudget::with_limit(64 * 1024 * 1024);
    let parsed = parse_svg_image(
        svg(&format!("{text}{text}")).as_bytes(),
        &default_svg_fonts(),
        &budget,
    )
    .unwrap();
    assert_eq!(
        images(&parsed),
        1,
        "the second does not fit beside the first"
    );

    // The levels of nested documents add up, as usvg converts a nested
    // document within its `<image>`; those of siblings do not.
    on_guest_stack(|| {
        let icon = svg(&format!("{}{}", "<g>".repeat(40), "</g>".repeat(40)));
        let sheet = parse(&svg(&nesting(&icon).repeat(16))).unwrap();
        assert_eq!(images(&sheet), 16);
        let deep = |depth| svg(&format!("{}{}", "<g>".repeat(depth), "</g>".repeat(depth)));
        assert!(parse(&deep(MAX_NESTING - 1)).is_ok());
        let nested = parse(&svg(&nesting(&deep(MAX_NESTING - 1)))).unwrap();
        assert_eq!(images(&nested), 0);
        let nested = parse(&svg(&nesting(&deep(MAX_NESTING - 8)))).unwrap();
        assert_eq!(images(&nested), 1);
    });
}

#[test]
fn documents_without_an_svg_root_measure_nothing() {
    let doc = roxmltree::Document::parse("<html/>").unwrap();
    assert_eq!(conversion(&doc).unwrap().bytes, 0);
}

#[test]
fn nested_pattern_chains_are_bounded_before_conversion_and_cloning() {
    let chain = |links: usize| {
        let mut body = String::from(
            "<pattern id='p0' width='1' height='1'><rect width='1' height='1'/></pattern>",
        );
        for link in 1..links {
            body.push_str(&format!("<pattern id='p{link}' width='1' height='1'><rect width='1' height='1' fill='url(#p{})'/></pattern>",link-1));
        }
        body.push_str(&format!(
            "<rect width='1' height='1' fill='url(#p{})'/>",
            links - 1
        ));
        svg(&body)
    };
    assert!(parse(&chain(8)).is_ok());
    assert!(matches!(parse(&chain(200)), Err(SvgRefusal::Expansion)));
}
