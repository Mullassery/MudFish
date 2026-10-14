use criterion::{criterion_group, criterion_main, Criterion};
use mudfish_parser::adaptive::{fingerprint, relocate, RelocateOptions};
use scraper::{Html, Selector};
use std::hint::black_box;

/// A listing-style page made of product cards, grown until it is at least
/// `target_bytes`. This mirrors the shape the crawler actually meets (many
/// near-identical repeated blocks) and is the pathological case for
/// similarity scoring: the target's decoys are structurally near-identical.
fn synthetic_page(target_bytes: usize) -> String {
    let mut html = String::from(
        "<html><head><title>Bench Listing</title></head><body><main class=\"catalog\">",
    );
    let mut index = 0usize;
    while html.len() < target_bytes {
        html.push_str(&format!(
            "<article class=\"card\"><h2 class=\"name\">Product {index}</h2>\
             <p class=\"desc\">A synthetic description of product {index} long enough \
             to resemble a real listing blurb.</p>\
             <span class=\"price\" data-currency=\"USD\">${index}.99</span>\
             <a class=\"buy\" href=\"/item/{index}\">Buy</a></article>"
        ));
        index += 1;
    }
    html.push_str("</main></body></html>");
    html
}

/// Fingerprints one `.price` element from a minimal page that has the same
/// card structure as `synthetic_page`, so the benchmark measures a genuine
/// recovery against a large number of similar candidates.
fn bench_fingerprint() -> mudfish_parser::adaptive::ElementFingerprint {
    let html = "<html><body><main class=\"catalog\"><article class=\"card\">\
        <h2 class=\"name\">Product 0</h2>\
        <p class=\"desc\">A synthetic description of product 0 long enough to resemble \
        a real listing blurb.</p>\
        <span class=\"price\" data-currency=\"USD\">$0.99</span>\
        <a class=\"buy\" href=\"/item/0\">Buy</a></article></main></body></html>";
    let document = Html::parse_document(html);
    let selector = Selector::parse(".price").unwrap();
    fingerprint(document.select(&selector).next().unwrap())
}

fn bench_relocate(c: &mut Criterion) {
    let fp = bench_fingerprint();
    let small = Html::parse_document(&synthetic_page(20_000));
    let large_html = synthetic_page(2_000_000);
    let large_size = large_html.len();
    let large = Html::parse_document(&large_html);
    let options = RelocateOptions::default();

    // Report the actual fixture size once so the number in the ADR (§11
    // targets a 2 MB document) is verifiable rather than assumed.
    eprintln!("adaptive_relocate: large fixture is {large_size} bytes");

    c.bench_function("relocate_20kb_document", |b| {
        b.iter(|| {
            black_box(relocate(
                black_box(&fp),
                black_box(&small),
                black_box(&options),
            ))
        })
    });

    c.bench_function("relocate_2mb_document", |b| {
        b.iter(|| {
            black_box(relocate(
                black_box(&fp),
                black_box(&large),
                black_box(&options),
            ))
        })
    });
}

criterion_group!(benches, bench_relocate);
criterion_main!(benches);
