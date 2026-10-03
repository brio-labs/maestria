use std::error::Error;

use lopdf::{
    Dictionary, Object, Stream,
    content::{Content, Operation},
};

pub fn create_minimal_pdf(text: &[u8]) -> Result<Vec<u8>, Box<dyn Error>> {
    let mut doc = lopdf::Document::with_version("1.4");
    let pages_id = doc.new_object_id();
    let page_id = doc.new_object_id();
    let content_id = doc.new_object_id();
    let font_id = doc.new_object_id();

    let mut font_dict = Dictionary::new();
    font_dict.set("Type", Object::Name("Font".into()));
    font_dict.set("Subtype", Object::Name("Type1".into()));
    font_dict.set("BaseFont", Object::Name("Courier".into()));
    doc.objects.insert(font_id, Object::Dictionary(font_dict));

    let content = Content {
        operations: vec![
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec![Object::Name("F1".into()), Object::Integer(12)]),
            Operation::new("Td", vec![Object::Integer(72), Object::Integer(700)]),
            Operation::new(
                "Tj",
                vec![Object::String(text.to_vec(), lopdf::StringFormat::Literal)],
            ),
            Operation::new("ET", vec![]),
        ],
    };
    doc.objects.insert(
        content_id,
        Object::Stream(Stream::new(Dictionary::new(), content.encode()?)),
    );

    let mut resources = Dictionary::new();
    let mut fonts = Dictionary::new();
    fonts.set("F1", Object::Reference(font_id));
    resources.set("Font", Object::Dictionary(fonts));

    let mut page = Dictionary::new();
    page.set("Type", Object::Name("Page".into()));
    page.set("Parent", Object::Reference(pages_id));
    page.set(
        "MediaBox",
        Object::Array(vec![
            Object::Integer(0),
            Object::Integer(0),
            Object::Integer(612),
            Object::Integer(792),
        ]),
    );
    page.set("Contents", Object::Reference(content_id));
    page.set("Resources", Object::Dictionary(resources));
    doc.objects.insert(page_id, Object::Dictionary(page));

    let mut pages = Dictionary::new();
    pages.set("Type", Object::Name("Pages".into()));
    pages.set("Kids", Object::Array(vec![Object::Reference(page_id)]));
    pages.set("Count", Object::Integer(1));
    doc.objects.insert(pages_id, Object::Dictionary(pages));

    let catalog_id = doc.new_object_id();
    let mut catalog = Dictionary::new();
    catalog.set("Type", Object::Name("Catalog".into()));
    catalog.set("Pages", Object::Reference(pages_id));
    doc.objects.insert(catalog_id, Object::Dictionary(catalog));
    doc.trailer.set("Root", Object::Reference(catalog_id));

    let mut output = Vec::new();
    doc.save_to(&mut output)?;
    Ok(output)
}
