use kuchiki::parse_html;
use kuchiki::traits::TendrilSink;

pub fn generate_render_tree(html: &str) -> crate::RenderTree {
    let parser = parse_html();
    let document = parser.one(html);

    for edge in document.traverse() {
        println!("{:?}", edge);
    }

    crate::RenderTree::new()
}
