use pbi_lens_lib::pbix::parse_report;

fn main() {
    for path in std::env::args().skip(1) {
        match parse_report(std::path::Path::new(&path)) {
            Ok(r) => println!(
                "{}: {} pages, {} visuals, {} tables, {} sources, {} queries, {} entries in {} ms",
                r.name,
                r.pages.len(),
                r.visual_count,
                r.tables.len(),
                r.sources.len(),
                r.queries.len(),
                r.entries.len(),
                r.parse_ms
            ),
            Err(e) => println!("{path}: ERROR {e}"),
        }
    }
}
