//! CPU admission regressions, exercised on the guest's stack size.

use super::{OPEN, on_guest_stack, parse};
use crate::svg_bounds::SvgRefusal;
use crate::svg_expansion::expand_entities;

#[test]
fn repeated_entity_lookups_are_refused_before_xml_parsing() {
    on_guest_stack(|| {
        let declarations: String = (0..1_000).map(|i| format!("<!ENTITY e{i} ''>")).collect();
        let source = format!(
            "<!DOCTYPE svg [{declarations}]>{OPEN}<text>{}</text></svg>",
            "&e999;".repeat(10_000)
        );
        let started = std::time::Instant::now();
        assert!(expand_entities(&source).is_none());
        assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
        assert!(started.elapsed().as_secs() < 2);
    });
}

#[test]
fn nested_empty_entity_references_still_cost_work() {
    on_guest_stack(|| {
        let name = "a".repeat(100);
        let source = format!(
            "<!DOCTYPE svg [<!ENTITY {name} ''><!ENTITY outer '{}'>]>{OPEN}<text>{}</text></svg>",
            format!("&{name};").repeat(100),
            "&outer;".repeat(200)
        );
        assert!(expand_entities(&source).is_none());
        assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
    });
}

#[test]
fn ordinary_entity_lookups_remain_admitted() {
    let source = format!(
        "<!DOCTYPE svg [<!ENTITY color 'blue'><!ENTITY label 'SVG'>]>{OPEN}<rect width='10' height='10' fill='&color;'/><text y='20'>&label;</text></svg>"
    );
    assert!(parse(&source).is_ok());
}

#[test]
fn descendant_selector_backtracking_is_stopped_during_preflight() {
    on_guest_stack(|| {
        let source = super::svg(&format!(
            "<style>x g g g g rect {{ fill: red }}</style>{}<rect width='1' height='1'/>{}",
            "<g>".repeat(120),
            "</g>".repeat(120)
        ));
        let started = std::time::Instant::now();
        assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
        assert!(started.elapsed().as_secs() < 2);
    });
}

#[test]
fn stylesheet_matching_work_counts_use_copies() {
    let rules: String = (0..100).map(|i| format!(".c{i} {{ fill: red }}")).collect();
    let source = super::svg(&format!(
        "<style>{rules}</style><g id='sprite'>{}</g>{}",
        "<rect width='1' height='1' class='ordinary'/>".repeat(50),
        "<use href='#sprite'/>".repeat(500)
    ));
    assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
}

#[test]
fn ordinary_diagram_styles_remain_admitted() {
    let source = super::svg(
        "<style>.diagram .node > rect { fill: blue; stroke: black } .edge + .label {fill:red}</style><g class='diagram'><g class='node'><rect width='20' height='10'/></g><path class='edge' d='M0 0L20 20'/><text class='label' y='30'>Diagram</text></g>",
    );
    assert!(parse(&source).is_ok());
}

#[test]
fn repeated_first_child_checks_are_charged_in_copies() {
    let selector = ":first-child".repeat(300);
    let rules = format!("{selector} {{ fill: blue }}").repeat(100);
    let source = super::svg(&format!(
        "<style>{rules}</style><g id='sprite'><rect width='1' height='1'/></g>{}",
        "<use href='#sprite'/>".repeat(100)
    ));
    assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
}

#[test]
fn failing_pseudo_class_rules_cannot_hide_matching_work() {
    let selector = format!("{}:hover {{ fill: blue }}", ":first-child".repeat(300));
    let source = super::svg(&format!(
        "<style>{}</style>{}",
        selector.repeat(200),
        "<g><rect width='1' height='1'/></g>".repeat(1_000)
    ));
    assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
}

#[test]
fn repeated_stylesheet_sorting_is_refused_before_parsing() {
    for sheets in [
        "<style>rect { fill: blue }</style>".repeat(1_000),
        format!(
            "<style>{}</style>{}",
            ".node { fill: blue }".repeat(1_000),
            "<style> </style>".repeat(1_000)
        ),
    ] {
        let source = super::svg(&sheets);
        assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
    }
}

#[test]
fn recursive_selector_comments_are_refused_before_parsing() {
    on_guest_stack(|| {
        let source = super::svg(&format!(
            "<style>rect{} {{ fill: blue }}</style><rect width='1' height='1'/>",
            "/**/".repeat(100_000)
        ));
        assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
    });
}

#[test]
fn tref_lookup_work_counts_expanded_copies_and_unknown_xml() {
    on_guest_stack(|| {
        for href in ["#last", "#missing"] {
            let source = super::svg(&format!(
                "<unknown>{}<g id='last'/></unknown><text id='label'><tref href='{href}'/></text>{}",
                "<unknown id='unused'/>".repeat(1_000),
                "<use href='#label'/>".repeat(300)
            ));
            let started = std::time::Instant::now();
            assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
            assert!(started.elapsed().as_secs() < 2);
        }
    });
}

#[test]
fn filter_collection_work_is_quadratic_across_copies() {
    on_guest_stack(|| {
        // About 16 MiB of estimated conversion memory, below the image budget,
        // but millions of pointer comparisons in usvg's filter collection.
        let source = super::svg(&format!(
            "<rect id='sprite' width='1' height='1' filter='{}'/>{}",
            "blur(0) ".repeat(100),
            "<use href='#sprite'/>".repeat(50)
        ));
        let started = std::time::Instant::now();
        assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
        assert!(started.elapsed().as_secs() < 2);
    });
}

#[test]
fn ordinary_tref_and_filter_functions_remain_admitted() {
    let source = super::svg(
        "<defs><text id='label'>Diagram</text></defs><text y='20'><tref href='#label'/></text><rect width='20' height='20' filter='blur(1) brightness(0.5)'/>",
    );
    assert!(parse(&source).is_ok());
}

#[test]
fn inherited_filter_functions_are_counted_for_every_child() {
    on_guest_stack(|| {
        let source = super::svg(&format!(
            "<g filter='{}'>{}</g>",
            "blur(0) ".repeat(100),
            "<rect width='1' height='1' filter='inherit'/>".repeat(100)
        ));
        let started = std::time::Instant::now();
        assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
        assert!(started.elapsed().as_secs() < 2);
    });
}

#[test]
fn filter_functions_do_not_implicitly_inherit() {
    let source = super::svg(&format!(
        "<g filter='{}'>{}</g>",
        "blur(0) ".repeat(10),
        "<rect width='1' height='1'/>".repeat(150)
    ));
    assert!(parse(&source).is_ok());
}

#[test]
fn inherited_filter_urls_include_every_target_and_copy() {
    on_guest_stack(|| {
        let source = super::svg(&format!(
            "<defs><filter id='f'><feGaussianBlur stdDeviation='0'/></filter></defs><g filter='{}'>{}</g>",
            "url(#f) ".repeat(100),
            "<rect width='1' height='1' filter='inherit'/>".repeat(100)
        ));
        let doc = crate::svg_root_size::parse_xml(&source).unwrap();
        assert!(super::conversion(&doc).unwrap().work > 2_000_000);
        let started = std::time::Instant::now();
        assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
        assert!(started.elapsed().as_secs() < 2);
    });
}

#[test]
fn ordinary_explicit_filter_inheritance_is_admitted() {
    for filter in ["blur(1)", "url(#f)"] {
        let source = super::svg(&format!(
            "<defs><filter id='f'><feGaussianBlur stdDeviation='1'/></filter></defs><g filter='{filter}'><rect width='10' height='10' filter='inherit'/></g>"
        ));
        assert!(parse(&source).is_ok());
    }
}

#[test]
fn filter_inheritance_from_stylesheets_and_use_instances_is_counted() {
    for function in ["blur(0) ", "url(#f) "] {
        let filter = function.repeat(100);
        let instance_filter = function.repeat(10);
        for body in [
            format!(
                "<style>.inherited {{ filter: inherit }}</style><g filter='{filter}'>{}</g>",
                "<rect width='1' height='1' class='inherited'/>".repeat(100)
            ),
            format!(
                "<defs><rect id='sprite' width='1' height='1' filter='inherit'/></defs>{}",
                format!("<use href='#sprite' filter='{instance_filter}'/>").repeat(100)
            ),
        ] {
            let source = super::svg(&format!(
                "<defs><filter id='f'><feGaussianBlur stdDeviation='0'/></filter></defs>{body}"
            ));
            let doc = crate::svg_root_size::parse_xml(&source).unwrap();
            assert!(super::conversion(&doc).unwrap().work > 2_000_000);
            assert!(matches!(parse(&source), Err(SvgRefusal::Expansion)));
        }
    }
}

fn parse_text_path(source: &str) -> Result<crate::node::SvgImageData, SvgRefusal> {
    let mut fonts = usvg::fontdb::Database::new();
    fonts.load_font_data(
        include_bytes!("../../../assets/test-fonts/LiberationSans-Regular.ttf").to_vec(),
    );
    fonts.set_serif_family("Liberation Sans");
    fonts.set_sans_serif_family("Liberation Sans");
    crate::svg_bounds::parse_svg_image(
        source.as_bytes(),
        &std::sync::Arc::new(fonts),
        &crate::DecodedImageBudget::default(),
    )
}

#[test]
fn text_path_measurement_bounds_resolved_geometry_and_tolerance() {
    on_guest_stack(|| {
        for (shape, transform, offset) in [
            ("<path id='p' d='M0 0L1e30 0'/>", "", "0"),
            ("<path id='p' d='M0 0C0 1e30 1e30 1e30 1e30 0'/>", "", "50%"),
            ("<circle id='p' r='1e30'/>", "", "0"),
            (
                "<path id='p' d='M0 0C0 100 100 100 100 0' transform='scale(1e20)'/>",
                "",
                "0",
            ),
            (
                "<path id='p' d='M0 0C0 100 100 100 100 0'/>",
                "scale(1e20)",
                "0",
            ),
        ] {
            let source = super::svg(&format!(
                "<defs>{shape}</defs><text transform='{transform}'><textPath href='#p' startOffset='{offset}'>Bounded</textPath></text>"
            ));
            let started = std::time::Instant::now();
            assert!(parse_text_path(&source).is_err(), "{source}");
            assert!(started.elapsed().as_secs() < 2, "{source}");
        }
    });
}

#[test]
fn ordinary_text_paths_still_produce_glyph_outlines() {
    for offset in ["0", "20%"] {
        let source = super::svg(&format!(
            "<defs><path id='p' d='M5 50C25 10 75 10 95 50'/></defs><text font-size='10'><textPath href='#p' startOffset='{offset}'>Diagram</textPath></text>"
        ));
        let parsed = parse_text_path(&source).unwrap();
        assert!(!parsed.tree.root().children().is_empty());
        assert!(parsed.tree.root().bounding_box().width() > 10.0);
    }
}

#[test]
fn text_path_work_allowance_is_shared_across_use_copies() {
    on_guest_stack(|| {
        let source = super::svg(&format!(
            "<defs><path id='p' d='M5 50C25 10 75 10 95 50'/><text id='t' font-size='1'><textPath href='#p'>Repeated path measurement</textPath></text></defs>{}",
            "<use href='#t'/>".repeat(400)
        ));
        let started = std::time::Instant::now();
        // Invalid is returned by usvg conversion, after memory/expansion admission.
        assert!(matches!(parse_text_path(&source), Err(SvgRefusal::Invalid)));
        assert!(started.elapsed().as_secs() < 2);
    });
}
