//! Retention and bounds of the decoded image cache.
use super::{ImageCache, ImageRetention, MAX_CACHED_IMAGES};
use crate::DecodedImageBudget;
use crate::image_request::ImageKey;
use crate::node::{ImageData, RasterImageData};
use std::sync::Arc;

fn raster(width: u32, height: u32) -> ImageData {
    let bytes = width as usize * height as usize * 4;
    ImageData::Raster(RasterImageData::new(
        width,
        height,
        Arc::new(vec![0; bytes]),
    ))
}

fn key(name: &str) -> ImageKey {
    ImageKey::no_cors(format!("https://cdn.test/{name}"))
}

#[test]
fn stored_images_outlive_their_holders() {
    let budget = DecodedImageBudget::default();
    let mut cache = ImageCache::default();
    cache.insert(key("a"), &raster(1, 1), ImageRetention::Store, &budget);
    assert!(cache.contains_key(&key("a")));
}

#[test]
fn held_images_are_available_until_the_last_holder_lets_go() {
    let budget = DecodedImageBudget::default();
    let mut cache = ImageCache::default();
    let mut image = raster(2, 1);
    if let ImageData::Raster(raster) = &mut image {
        raster.origin_clean = true;
    }
    cache.insert(key("a"), &image, ImageRetention::WhileHeld, &budget);
    let Some(ImageData::Raster(shared)) = cache.get(&key("a")) else {
        panic!("held image is available");
    };
    assert_eq!(
        (shared.width, shared.height, shared.origin_clean),
        (2, 1, true)
    );
    drop(image);
    assert!(cache.contains_key(&key("a")), "the copy still holds it");
    drop(shared);
    assert!(cache.get(&key("a")).is_none());
}

#[test]
fn released_entries_free_their_slot() {
    let budget = DecodedImageBudget::default();
    let mut cache = ImageCache::default();
    let held = raster(1, 1);
    cache.insert(key("held"), &held, ImageRetention::WhileHeld, &budget);
    for index in 1..MAX_CACHED_IMAGES {
        cache.insert(
            key(&index.to_string()),
            &raster(1, 1),
            ImageRetention::Store,
            &budget,
        );
    }
    cache.insert(
        key("refused"),
        &raster(1, 1),
        ImageRetention::Store,
        &budget,
    );
    assert!(
        !cache.contains_key(&key("refused")),
        "a held entry takes a slot"
    );
    drop(held);
    cache.insert(
        key("accepted"),
        &raster(1, 1),
        ImageRetention::Store,
        &budget,
    );
    assert!(cache.contains_key(&key("accepted")));
    assert_eq!(cache.len(), MAX_CACHED_IMAGES);
}

#[test]
fn an_image_nothing_can_hold_is_not_kept() {
    let budget = DecodedImageBudget::default();
    let mut cache = ImageCache::default();
    cache.insert(
        key("a"),
        &ImageData::None,
        ImageRetention::WhileHeld,
        &budget,
    );
    assert!(cache.is_empty());
}

#[cfg(feature = "svg")]
#[test]
fn held_svg_trees_are_available_while_held() {
    let fonts = crate::util::default_svg_fonts();
    let svg = crate::svg_bounds::parse_svg_image(
        b"<svg xmlns='http://www.w3.org/2000/svg' width='3' height='4'/>",
        &fonts,
        &DecodedImageBudget::default(),
    )
    .expect("a valid SVG");
    let image = ImageData::Svg(svg);
    let budget = DecodedImageBudget::default();
    let mut cache = ImageCache::default();
    cache.insert(key("a"), &image, ImageRetention::WhileHeld, &budget);
    assert!(matches!(cache.get(&key("a")), Some(ImageData::Svg(_))));
    drop(image);
    assert!(cache.get(&key("a")).is_none());
}

#[test]
fn the_cache_is_bounded() {
    let budget = DecodedImageBudget::default();
    let mut cache = ImageCache::default();
    for index in 0..(MAX_CACHED_IMAGES + 10) {
        cache.insert(
            key(&index.to_string()),
            &raster(1, 1),
            ImageRetention::Store,
            &budget,
        );
    }
    assert_eq!(cache.len(), MAX_CACHED_IMAGES);
    cache.clear();
    cache.insert(
        key("large"),
        &raster(4096, 4097),
        ImageRetention::Store,
        &budget,
    );
    assert!(cache.is_empty(), "one image over the byte budget");
}

#[test]
fn stored_images_give_way_before_a_decode_is_refused() {
    let budget = DecodedImageBudget::with_limit(100);
    let mut cache = ImageCache::default();
    let image = raster(4, 4);
    let ImageData::Raster(pixels) = &image else {
        unreachable!("a raster");
    };
    budget.reserve(64).unwrap().track_raster(pixels);
    cache.insert(key("a"), &image, ImageRetention::Store, &budget);
    drop(image);
    assert!(cache.contains_key(&key("a")), "pinned by the budget");
    assert!(budget.reserve(64).is_some(), "the pin gave way");
    assert!(!cache.contains_key(&key("a")));
}
