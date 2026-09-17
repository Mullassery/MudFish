use criterion::{criterion_group, criterion_main, Criterion};
use mudfish_core::{fingerprint, normalize_url, NormalizationOptions};
use std::hint::black_box;
use url::Url;

fn bench_normalize(c: &mut Criterion) {
    let opts = NormalizationOptions::default();
    let urls: Vec<Url> = vec![
        "https://example.com/page?utm_source=x&utm_medium=y&id=42",
        "https://EXAMPLE.com:443/a//b///c/",
        "https://example.com/search?q=rust&sort=recent",
        "https://example.com/blog/post-title-here#comments",
    ]
    .into_iter()
    .map(|s| Url::parse(s).unwrap())
    .collect();

    c.bench_function("normalize_url", |b| {
        b.iter(|| {
            for u in &urls {
                black_box(normalize_url(black_box(u), &opts));
            }
        })
    });

    let normalized: Vec<Url> = urls.iter().map(|u| normalize_url(u, &opts)).collect();
    c.bench_function("fingerprint", |b| {
        b.iter(|| {
            for u in &normalized {
                black_box(fingerprint(black_box(u)));
            }
        })
    });
}

criterion_group!(benches, bench_normalize);
criterion_main!(benches);
