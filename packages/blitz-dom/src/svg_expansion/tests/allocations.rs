//! The measure bounds what usvg allocates: the peak of each conversion is
//! counted by the test binary's allocator (see `crate::test_allocator`).

use super::{OPEN, svg};
use crate::svg_expansion::{conversion, measure_bytes};
use crate::test_allocator::peak_held;

/// The most bytes building and converting `source` holds at once.
fn peak(source: &str) -> usize {
    let doc = crate::svg_root_size::parse_xml(source).unwrap();
    let options = usvg::Options {
        fontdb: crate::util::default_svg_fonts(),
        ..Default::default()
    };
    // The font database is loaded before tracking, as documents share it.
    let _ = options.fontdb.len();
    peak_held(|| usvg::Tree::from_xmltree(&doc, &options))
}

#[test]
fn the_measure_bounds_what_usvg_allocates() {
    let points: String = (0..500).map(|i| format!(" L{i} {}", i % 7)).collect();
    let stops = "<stop offset='0'/><stop offset='1' stop-color='red'/>".repeat(50);
    let documents = [
        svg("<rect width='10' height='10'/>"),
        svg(&"<rect width='1' height='1' fill='red' stroke='blue'/>".repeat(2000)),
        svg(&format!("<path d='M0 0{points}' stroke='black'/>").repeat(50)),
        svg(&"<circle r='1e30'/>".repeat(4)),
        svg(&format!("<text y='20'>{}</text>", "Hello, world. ".repeat(2000))),
        svg(&format!(
            "<text id='t'>{}</text>{}",
            "abc ".repeat(100),
            "<text y='9'><tref href='#t'/></text>".repeat(50)
        )),
        svg(&format!(
            "<linearGradient id='g'>{stops}</linearGradient>{}",
            "<rect width='1' height='1' fill='url(#g)'/>".repeat(500)
        )),
        svg(&format!(
            "<pattern id='p' width='4' height='4'>{}</pattern><g fill='url(#p)'>{}</g>",
            "<rect width='1' height='1'/>".repeat(20),
            "<rect width='9' height='9'/>".repeat(200)
        )),
        svg(&format!(
            "<marker id='m'><path d='M0 0{points}'/></marker><path d='M0 0{points}' marker-mid='url(#m)'/>"
        )),
        svg(&format!(
            "<clipPath id='c'><path d='M0 0{points}'/></clipPath>{}",
            "<rect width='9' height='9' clip-path='url(#c)'/>".repeat(100)
        )),
        svg(&format!(
            "<style>rect {{ font-family: {} }}</style>{}",
            "a".repeat(2000),
            "<rect width='1' height='1'/>".repeat(500)
        )),
        {
            let mut body = String::from("<defs><rect id='l0' width='1' height='1'/>");
            for level in 1..=3 {
                body.push_str(&format!("<g id='l{level}'>"));
                body.push_str(&format!("<use href='#l{}'/>", level - 1).repeat(10));
                body.push_str("</g>");
            }
            body.push_str("</defs><use href='#l3'/>");
            svg(&body)
        },
        format!(
            "{}<filter id='f'><feGaussianBlur stdDeviation='2'/><feOffset dx='1'/></filter>{}</svg>",
            OPEN,
            "<rect width='9' height='9' filter='url(#f)'/>".repeat(200)
        ),
        svg(&format!(
            "<filter id='f'><feFlood/></filter>{}",
            "<rect width='9' height='9' filter='url(#f)'/>".repeat(500)
        )),
        svg(&format!(
            "<rect id='r' width='4' height='4'/><filter id='f'><feImage href='#r'/><feMerge><feMergeNode/><feMergeNode in='SourceGraphic'/></feMerge></filter>{}",
            "<rect width='9' height='9' filter='url(#f)'/>".repeat(200)
        )),
        svg(&format!(
            "<symbol id='s' viewBox='0 0 4 4'><rect width='4' height='4'/></symbol>{}",
            "<use href='#s' width='8' height='8'/>".repeat(500)
        )),
        svg(&"<svg x='1' width='5' height='5' viewBox='0 0 1 1'><rect width='1' height='1'/></svg>".repeat(500)),
        svg(&format!(
            "<mask id='m' maskContentUnits='objectBoundingBox'><rect width='1' height='1' fill='white'/></mask>{}",
            "<rect width='9' height='9' mask='url(#m)'/>".repeat(300)
        )),
        svg(&format!(
            "<path id='p' d='M0 0{points}'/><text>{}</text>",
            "<textPath href='#p'>Along the path</textPath>"
        )),
        svg(&format!(
            "<path id='p' d='M0 0{points}'/>{}",
            "<text><textPath href='#p'>ab</textPath></text>".repeat(100)
        )),
        svg(&format!(
            "<text y='9' text-decoration='underline'>{}</text>",
            "<tspan fill='red'>a</tspan><tspan dx='1'>b</tspan>".repeat(500)
        )),
        svg(&format!("<switch>{}</switch>", "<rect width='1' height='1'/>".repeat(500))),
        svg(&format!(
            "<pattern id='p' width='1' height='1' patternContentUnits='objectBoundingBox'><circle r='0.5'/></pattern>{}",
            "<rect width='9' height='9' fill='url(#p)' stroke='url(#p)'/>".repeat(300)
        )),
        svg(&format!(
            "<marker id='m' viewBox='0 0 4 4' markerWidth='4' markerHeight='4'><circle cx='2' cy='2' r='2'/></marker>{}",
            format!("<polyline points='{}' marker-start='url(#m)' marker-mid='url(#m)' marker-end='url(#m)'/>", "1 2 3 4 ".repeat(100)).repeat(5)
        )),
        svg(&format!(
            "<radialGradient id='a'>{stops}</radialGradient><radialGradient id='b' href='#a' gradientUnits='userSpaceOnUse'/>{}",
            "<ellipse rx='5' ry='3' fill='url(#b)' stroke='url(#a)'/>".repeat(300)
        )),
        svg(&format!(
            "<g stroke='black' stroke-dasharray='{}'>{}</g>",
            "1 ".repeat(5000),
            "<path d='M0 0h1'/>".repeat(500)
        )),
        svg(&format!(
            "<rect id='r' width='1' height='1' filter='{}'/>{}",
            "blur(1) ".repeat(500),
            "<use href='#r'/>".repeat(20)
        )),
        svg(&format!(
            "<pattern id='p' width='1' height='1'>{}</pattern><text x='{}' fill='url(#p)'>{}</text>",
            "<rect width='1' height='1'/>".repeat(50),
            (0..300).map(|x| format!("{x} ")).collect::<String>(),
            "x".repeat(300)
        )),
        svg(&format!(
            "<linearGradient id='g'><stop offset='0'/><stop offset='1' stop-color='red'/></linearGradient><text y='20' fill='url(#g)'>{}</text>",
            format!("<tspan>{}</tspan>", "Some words of text. ".repeat(6)).repeat(50)
        )),
        svg(&format!(
            "<style>{}</style><rect width='1' height='1'/>",
            "a{}b,c{d:e}".repeat(5_000)
        )),
        svg(&format!(
            "<g font-family='{}'>{}</g>",
            "a,".repeat(2000),
            "<text>x</text>".repeat(100)
        )),
    ];
    for source in &documents {
        measure_bounds_itself(source);
        let doc = crate::svg_root_size::parse_xml(source).unwrap();
        let reserved = conversion(&doc).unwrap().bytes;
        let peak = peak(source);
        assert!(
            peak <= reserved,
            "{peak} > {reserved} for {}",
            &source[..source.len().min(160)]
        );
    }
}

/// What the measure holds while it runs stays within what it reserves.
fn measure_bounds_itself(source: &str) {
    let doc = crate::svg_root_size::parse_xml(source).unwrap();
    let measuring = peak_held(|| conversion(&doc));
    assert!(
        measuring <= measure_bytes(&doc),
        "measuring {measuring} > {} for {}",
        measure_bytes(&doc),
        &source[..source.len().min(160)]
    );
}

/// References and declarations matched by every element are refused before
/// the measure holds more than it reserves.
#[test]
fn the_measure_bounds_its_own_memory() {
    let filters = svg(&format!(
        "<style>g {{ filter: {} }}</style><filter id='f'/>{}",
        "url(#f) ".repeat(10_000),
        "<g/>".repeat(5000)
    ));
    let markers = svg(&format!(
        "<style>rect {{ {} }}</style><rect width='1' height='1'/>",
        "marker: url(#m); ".repeat(20_000)
    ));
    for source in [filters, markers] {
        measure_bounds_itself(&source);
        let doc = crate::svg_root_size::parse_xml(&source).unwrap();
        assert!(conversion(&doc).is_none());
    }
    // Each selector of a list gets its own copy of the block: counted
    // before it is parsed, and refused by the budget when too large.
    let shared = |selectors: usize, declarations: usize| {
        svg(&format!(
            "<style>{} {{ {} }}</style><rect width='1' height='1'/>",
            vec!["a"; selectors].join(","),
            "fill: red; ".repeat(declarations)
        ))
    };
    measure_bounds_itself(&shared(2_000, 200));
    let parsed = crate::svg_bounds::parse_svg_image(
        shared(20_000, 2_000).as_bytes(),
        &crate::util::default_svg_fonts(),
        &crate::DecodedImageBudget::default(),
    );
    assert!(matches!(parsed, Err(crate::svg_bounds::SvgRefusal::Budget)));
}
