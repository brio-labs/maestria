use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};

pub fn render_markdown(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::all());
    let mut output = String::new();
    let mut links: Vec<Option<String>> = Vec::new();
    for event in parser {
        match event {
            Event::Start(Tag::Paragraph) => output.push_str("<p>"),
            Event::End(TagEnd::Paragraph) => output.push_str("</p>"),
            Event::Start(Tag::Heading { level, .. }) => {
                output.push_str(&format!("<h{}>", heading_number(level)))
            }
            Event::End(TagEnd::Heading(level)) => {
                output.push_str(&format!("</h{}>", heading_number(level)))
            }
            Event::Start(Tag::BlockQuote(_)) => output.push_str("<blockquote>"),
            Event::End(TagEnd::BlockQuote(_)) => output.push_str("</blockquote>"),
            Event::Start(Tag::List(Some(start))) => {
                output.push_str(&format!("<ol start=\"{start}\">"))
            }
            Event::Start(Tag::List(None)) => output.push_str("<ul>"),
            Event::End(TagEnd::List(true)) => output.push_str("</ol>"),
            Event::End(TagEnd::List(false)) => output.push_str("</ul>"),
            Event::Start(Tag::Item) => output.push_str("<li>"),
            Event::End(TagEnd::Item) => output.push_str("</li>"),
            Event::Start(Tag::Emphasis) => output.push_str("<em>"),
            Event::End(TagEnd::Emphasis) => output.push_str("</em>"),
            Event::Start(Tag::Strong) => output.push_str("<strong>"),
            Event::End(TagEnd::Strong) => output.push_str("</strong>"),
            Event::Start(Tag::Strikethrough) => output.push_str("<del>"),
            Event::End(TagEnd::Strikethrough) => output.push_str("</del>"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                let safe_destination = safe_link(&dest_url);
                if let Some(destination) = safe_destination.as_ref() {
                    output.push_str("<a href=\"");
                    output.push_str(&escape(destination));
                    output.push_str("\">");
                }
                links.push(safe_destination);
            }
            Event::End(TagEnd::Link) => {
                if let Some(Some(_)) = links.pop() {
                    output.push_str("</a>");
                }
            }
            Event::Start(Tag::Image { .. }) => {}
            Event::End(TagEnd::Image) => {}
            Event::Start(Tag::CodeBlock(kind)) => {
                output.push_str("<pre><code");
                if let CodeBlockKind::Fenced(language) = kind
                    && !language.is_empty()
                {
                    output.push_str(" class=\"language-");
                    output.push_str(&escape(&language));
                    output.push('"');
                }
                output.push('>');
            }
            Event::End(TagEnd::CodeBlock) => output.push_str("</code></pre>"),
            Event::Code(code) => {
                output.push_str("<code>");
                output.push_str(&escape(&code));
                output.push_str("</code>");
            }
            Event::Text(text) => output.push_str(&escape(&text)),
            Event::FootnoteReference(reference) => {
                output.push_str("<sup>");
                output.push_str(&escape(&reference));
                output.push_str("</sup>");
            }
            Event::SoftBreak => output.push('\n'),
            Event::HardBreak => output.push_str("<br />"),
            Event::Rule => output.push_str("<hr />"),
            Event::TaskListMarker(checked) => {
                let checkbox = if checked {
                    "<input type=\"checkbox\" checked disabled />"
                } else {
                    "<input type=\"checkbox\" disabled />"
                };
                output.push_str(checkbox);
            }
            Event::InlineHtml(_)
            | Event::Html(_)
            | Event::InlineMath(_)
            | Event::DisplayMath(_) => {}
            _ => {}
        }
    }
    output
}

fn heading_number(level: HeadingLevel) -> u8 {
    match level {
        HeadingLevel::H1 => 1,
        HeadingLevel::H2 => 2,
        HeadingLevel::H3 => 3,
        HeadingLevel::H4 => 4,
        HeadingLevel::H5 => 5,
        HeadingLevel::H6 => 6,
    }
}
fn safe_link(destination: &str) -> Option<String> {
    let lower = destination.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || (!lower.contains(':') && !destination.starts_with("//"))
    {
        Some(destination.to_owned())
    } else {
        None
    }
}
fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#x27;")
}

#[cfg(test)]
mod tests {
    use super::render_markdown;
    #[test]
    fn unsafe_html_and_links_are_discarded() {
        let html = render_markdown(
            "<script>alert(1)</script>\n[bad](javascript:alert(1)) [ok](https://example.com)",
        );
        assert!(!html.contains("<script>"));
        assert!(!html.contains("javascript:"));
        assert!(html.contains("https://example.com"));
    }
}
