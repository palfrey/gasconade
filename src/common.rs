use mustache;

pub fn render_to_response(path: &str, data: &mustache::Data) -> Vec<u8> {
    let template = mustache::compile_path(path).unwrap_or_else(|_| panic!("working template for {}", path));
    let mut buffer: Vec<u8> = vec![];
    template.render_data(&mut buffer, data).unwrap();
    buffer
}
