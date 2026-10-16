//! SVG text shares the document's host-backed font authority.
use parley::fontique::{FallbackKey, FontProvider, ProvidedFamily, Script};
use std::{
    collections::BTreeSet,
    sync::{Arc, Mutex},
};
use usvg::{FontFamily, FontResolver, fontdb};

#[derive(Clone)]
pub struct SvgFontDb {
    pub(crate) database: Arc<fontdb::Database>,
    provider: Option<Arc<dyn FontProvider>>,
}
impl SvgFontDb {
    pub fn new(database: Arc<fontdb::Database>) -> Self {
        Self {
            database,
            provider: None,
        }
    }
    pub fn with_font_provider(mut self, provider: Arc<dyn FontProvider>) -> Self {
        self.provider = Some(provider);
        self
    }
    pub(crate) fn resolver(&self) -> FontResolver<'static> {
        let Some(provider) = &self.provider else {
            return FontResolver::default();
        };
        let loaded = Arc::new(Mutex::new(BTreeSet::new()));
        let named_loaded = loaded.clone();
        let named = provider.clone();
        let fallback = provider.clone();
        let select_fallback = FontResolver::default_fallback_selector();
        FontResolver {
            select_font: Box::new(move |font, database| {
                for family in font.families() {
                    let selected = match family {
                        FontFamily::Named(name) => {
                            if let Some(id) = query(font, fontdb::Family::Name(name), database) {
                                return Some(id);
                            }
                            let Some(family) = named.load_font(name) else {
                                continue;
                            };
                            let name = family.name.clone();
                            if let Some(id) = query(font, fontdb::Family::Name(&name), database) {
                                return Some(id);
                            }
                            load_family(database, family, &named_loaded);
                            query(font, fontdb::Family::Name(&name), database)
                        }
                        FontFamily::Serif => query(font, fontdb::Family::Serif, database),
                        FontFamily::SansSerif => query(font, fontdb::Family::SansSerif, database),
                        FontFamily::Cursive => query(font, fontdb::Family::Cursive, database),
                        FontFamily::Fantasy => query(font, fontdb::Family::Fantasy, database),
                        FontFamily::Monospace => query(font, fontdb::Family::Monospace, database),
                    };
                    if selected.is_some() {
                        return selected;
                    }
                }
                query(font, fontdb::Family::Serif, database)
            }),
            select_fallback: Box::new(move |character, used, database| {
                if let Some(id) = select_fallback(character, used, database) {
                    return Some(id);
                }
                let script =
                    icu_properties::CodePointMapData::<icu_properties::props::Script>::new()
                        .get(character);
                let names =
                    icu_properties::PropertyNamesShort::<icu_properties::props::Script>::new();
                let script: Script = names.get(script)?.parse().ok()?;
                let family = fallback.load_fallback(FallbackKey::new(script, None))?;
                load_family(database, family, &loaded);
                select_fallback(character, used, database)
            }),
        }
    }
}
type Loaded = Mutex<BTreeSet<(u64, u32)>>;
fn load_family(database: &mut Arc<fontdb::Database>, family: ProvidedFamily, loaded: &Loaded) {
    // Keep supplied defaults stable, including when the host resolves an alias.
    if database.faces().any(|face| {
        face.families
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case(&family.name))
    }) {
        return;
    }
    let Ok(mut loaded) = loaded.try_lock() else {
        return;
    };
    let database = Arc::make_mut(database);
    for font in family.fonts {
        let source = font.data.id();
        if font
            .indices
            .iter()
            .all(|index| loaded.contains(&(source, *index)))
        {
            continue;
        }
        let ids = database.load_font_source(fontdb::Source::Binary(Arc::new(font.data)));
        for id in ids {
            let info = database.face(id).cloned();
            database.remove_face(id);
            let Some(mut info) = info else { continue };
            if font.indices.contains(&info.index) && loaded.insert((source, info.index)) {
                let language = info
                    .families
                    .first()
                    .map(|(_, language)| *language)
                    .unwrap_or(fontdb::Language::English_UnitedStates);
                info.families = vec![(family.name.clone(), language)];
                database.push_face_info(info);
            }
        }
    }
}

fn query(
    font: &usvg::Font,
    family: fontdb::Family<'_>,
    database: &fontdb::Database,
) -> Option<fontdb::ID> {
    database.query(&fontdb::Query {
        families: &[family],
        weight: fontdb::Weight(font.weight()),
        stretch: font.stretch().into(),
        style: font.style().into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use parley::fontique::{Blob, ProvidedFont};
    use std::sync::atomic::{AtomicUsize, Ordering};
    const SANS: &[u8] = include_bytes!("../assets/test-fonts/LiberationSans-Regular.ttf");
    struct Provider(AtomicUsize);
    impl FontProvider for Provider {
        fn load_font(&self, family: &str) -> Option<ProvidedFamily> {
            self.0.fetch_add(1, Ordering::Relaxed);
            (family == "Host Alias").then(|| ProvidedFamily {
                name: "Host Canonical".into(),
                fonts: vec![ProvidedFont {
                    data: Blob::new(Arc::new(SANS)),
                    indices: vec![0],
                }],
            })
        }
    }
    struct Fallback;
    impl FontProvider for Fallback {
        fn load_font(&self, _: &str) -> Option<ProvidedFamily> {
            None
        }
        fn load_fallback(&self, _: FallbackKey) -> Option<ProvidedFamily> {
            Some(ProvidedFamily {
                name: "Fallback Canonical".into(),
                fonts: vec![ProvidedFont {
                    data: Blob::new(Arc::new(SANS)),
                    indices: vec![0],
                }],
            })
        }
    }
    #[test]
    fn repeated_unsupported_glyphs_do_not_duplicate_fallback_faces() {
        let mut database = fontdb::Database::new();
        database.load_font_data(SANS.to_vec());
        let used = database.faces().next().unwrap().id;
        let fonts = SvgFontDb::new(Arc::new(database)).with_font_provider(Arc::new(Fallback));
        let resolver = fonts.resolver();
        let mut database = fonts.database.clone();
        for _ in 0..32 {
            assert!((resolver.select_fallback)('𐍈', &[used], &mut database).is_none());
        }
        assert_eq!(database.len(), 2);
        assert!(
            database
                .faces()
                .any(|face| face.families[0].0 == "Fallback Canonical")
        );
    }
    fn parse(text: &str, fonts: &SvgFontDb) -> usvg::Tree {
        usvg::Tree::from_str(
            text,
            &usvg::Options {
                fontdb: fonts.database.clone(),
                font_resolver: fonts.resolver(),
                ..Default::default()
            },
        )
        .unwrap()
    }
    #[test]
    fn svg_named_fonts_load_lazily_and_resolved_aliases_render() {
        let provider = Arc::new(Provider(AtomicUsize::new(0)));
        let fonts =
            SvgFontDb::new(Arc::new(fontdb::Database::new())).with_font_provider(provider.clone());
        assert_eq!(provider.0.load(Ordering::Relaxed), 0);
        let tree = parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="60"><text x="0" y="40" font-family="Host Alias" font-size="30">Host font</text></svg>"#,
            &fonts,
        );
        assert!(tree.root().abs_bounding_box().width() > 50.0);
        assert_eq!(provider.0.load(Ordering::Relaxed), 1);
    }
    #[test]
    fn missing_svg_fonts_preserve_default_selection() {
        let fonts = crate::build_svg_font_db(&[(crate::FontRole::SansSerif, SANS)]);
        let expected = parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="60"><text x="0" y="40" font-family="sans-serif" font-size="30">Fallback</text></svg>"#,
            &fonts,
        );
        let provider = Arc::new(Provider(AtomicUsize::new(0)));
        let fonts = fonts.with_font_provider(provider.clone());
        let actual = parse(
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="300" height="60"><text x="0" y="40" font-family="Missing, sans-serif" font-size="30">Fallback</text></svg>"#,
            &fonts,
        );
        assert_eq!(
            actual.root().abs_bounding_box(),
            expected.root().abs_bounding_box()
        );
        assert_eq!(provider.0.load(Ordering::Relaxed), 1);
    }
}
