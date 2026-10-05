use std::collections::HashMap;
fn main() {
    let src = std::fs::read_to_string(std::env::args().nth(1).unwrap()).unwrap();
    let d = graphing_dsl::Document::parse(src);
    let t = std::time::Instant::now();
    for _ in 0..200 {
        std::hint::black_box(graphing_scene::build(d.diagram(), &HashMap::new()));
    }
    println!("build: {:?} per call", t.elapsed() / 200);
}
