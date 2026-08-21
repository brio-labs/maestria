use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::fmt::Write as _;
pub fn render_markdown(markdown: &str) -> String {
    let parser = Parser::new_ext(markdown, Options::all());
    let mut output = String::with_capacity(markdown.len().saturating_add(markdown.len() / 2));
    let mut links: Vec<Option<String>> = Vec::new();
    for event in parser {
        match event {
            Event::Start(Tag::Paragraph) => output.push_str("<p>"),
            Event::End(TagEnd::Paragraph) => output.push_str("</p>"),
            Event::Start(Tag::Heading { level, .. }) => push_heading_start(level, &mut output),
            Event::End(TagEnd::Heading(level)) => push_heading_end(level, &mut output),
            Event::Start(Tag::BlockQuote(_)) => output.push_str("<blockquote>"),
            Event::End(TagEnd::BlockQuote(_)) => output.push_str("</blockquote>"),
            Event::Start(Tag::List(Some(start))) => {
                let _ = write!(output, "<ol start=\"{start}\">");
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
                    escape_into(destination, &mut output);
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
                    escape_into(&language, &mut output);
                    output.push('"');
                }
                output.push('>');
            }
            Event::End(TagEnd::CodeBlock) => output.push_str("</code></pre>"),
            Event::Code(code) => {
                output.push_str("<code>");
                escape_into(&code, &mut output);
                output.push_str("</code>");
            }
            Event::Text(text) => escape_into(&text, &mut output),
            Event::FootnoteReference(reference) => {
                output.push_str("<sup>");
                escape_into(&reference, &mut output);
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

fn push_heading_start(level: HeadingLevel, output: &mut String) {
    match level {
        HeadingLevel::H1 => output.push_str("<h1>"),
        HeadingLevel::H2 => output.push_str("<h2>"),
        HeadingLevel::H3 => output.push_str("<h3>"),
        HeadingLevel::H4 => output.push_str("<h4>"),
        HeadingLevel::H5 => output.push_str("<h5>"),
        HeadingLevel::H6 => output.push_str("<h6>"),
    }
}

fn push_heading_end(level: HeadingLevel, output: &mut String) {
    match level {
        HeadingLevel::H1 => output.push_str("</h1>"),
        HeadingLevel::H2 => output.push_str("</h2>"),
        HeadingLevel::H3 => output.push_str("</h3>"),
        HeadingLevel::H4 => output.push_str("</h4>"),
        HeadingLevel::H5 => output.push_str("</h5>"),
        HeadingLevel::H6 => output.push_str("</h6>"),
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
fn escape_into(value: &str, output: &mut String) {
    let mut last = 0;
    for (i, byte) in value.bytes().enumerate() {
        let replacement = match byte {
            b'&' => "&amp;",
            b'<' => "&lt;",
            b'>' => "&gt;",
            b'"' => "&quot;",
            b'\'' => "&#x27;",
            _ => continue,
        };
        if last < i {
            output.push_str(&value[last..i]);
        }
        output.push_str(replacement);
        last = i + 1;
    }
    if last < value.len() {
        output.push_str(&value[last..]);
    }
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
