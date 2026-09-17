use criterion::{criterion_group, criterion_main, Criterion};
use mudfish_parser::parse_html;
use std::hint::black_box;
use url::Url;

/// A synthetic page shaped like a real category/listing page: nav, N
/// product-style links, and a footer — representative of what the crawler
/// spends most of its parsing time on.
fn synthetic_page(link_count: usize) -> String {
    let mut html = String::from(
        r#"<html><head><title>Synthetic Listing Page</title>
        <meta name="description" content="A synthetic benchmark fixture.">
        <link rel="canonical" href="https://example.com/listing">
        </head><body><nav>
        <a href="/">Home</a><a href="/about">About</a><a href="/contact">Contact</a>
        </nav><main>"#,
    );
    for i in 0..link_count {
        html.push_str(&format!(
            r#"<a href="/item/{i}" rel="nofollow">Item {i}</a><p>Some descriptive text about item {i} that a real listing page would contain.</p>"#
        ));
    }
    html.push_str("</main><footer><a href=\"/privacy\">Privacy</a></footer></body></html>");
    html
}

fn bench_parse_html(c: &mut Criterion) {
    let base = Url::parse("https://example.com/listing").unwrap();
    let small = synthetic_page(20);
    let large = synthetic_page(500);

    c.bench_function("parse_html_20_links", |b| {
        b.iter(|| black_box(parse_html(black_box(&small), &base)))
    });

    c.bench_function("parse_html_500_links", |b| {
        b.iter(|| black_box(parse_html(black_box(&large), &base)))
    });
}

criterion_group!(benches, bench_parse_html);
criterion_main!(benches);
