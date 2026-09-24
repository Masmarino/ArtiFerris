use std::borrow::Cow;
use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ammonia::{Builder, UrlRelative};
use artiferris_domain::readme::{ReadmeRendererPort, MAX_README_BYTES};
use async_trait::async_trait;
use pulldown_cmark::{html, Event, Options, Parser};
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;

/// Deeper than any real README nests blocks and spans; deeper input makes the HTML parser do quadratic work.
const MAX_NESTING_DEPTH: usize = 32;
/// Sanitizing costs a few microseconds per element, so this keeps a render around 100 ms in release builds.
const MAX_ELEMENTS: usize = 20_000;
/// Raw HTML is parsed by html5ever, whose cost grows quickly with unclosed elements, so it gets a budget of its own.
const MAX_RAW_HTML_TAGS: usize = 2_000;
const RENDER_TIMEOUT: Duration = Duration::from_millis(250);
const CONCURRENT_RENDERS: usize = 4;
const CACHE_MAX_ENTRIES: usize = 128;
const CACHE_MAX_BYTES: usize = 16 * 1024 * 1024;

/// Markdown through `pulldown-cmark`, then everything through an `ammonia` allowlist. The allowlist is the
/// only thing standing between a package author and every visitor's browser, so it is deliberately narrow:
/// no styles, no forms, no frames, no relative or `javascript:` links, and images only over https.
///
/// A README past the nesting, element or raw-HTML budget, or one that takes too long, is shown as escaped plain text
/// instead. Conversion runs on the blocking pool, a few at a time, and finished renders are cached by content.
pub struct PulldownAmmoniaReadmeRenderer {
    inner: Arc<Inner>,
}

struct Inner {
    sanitizer: Builder<'static>,
    cache: Mutex<RenderCache>,
    permits: Arc<Semaphore>,
    timeout: Duration,
}

impl PulldownAmmoniaReadmeRenderer {
    pub fn new() -> Self {
        let tags: HashSet<&'static str> = [
            "a", "blockquote", "br", "code", "del", "details", "em", "h1", "h2", "h3", "h4", "h5", "h6", "hr", "img", "li", "ol", "p", "pre", "strong",
            "sub", "summary", "sup", "table", "tbody", "td", "th", "thead", "tr", "ul",
        ]
        .into_iter()
        .collect();
        let tag_attributes: HashMap<&'static str, HashSet<&'static str>> =
            HashMap::from([("a", ["href", "title"].into_iter().collect()), ("img", ["src", "alt", "title"].into_iter().collect())]);

        let mut sanitizer = Builder::empty();
        sanitizer
            .tags(tags)
            .tag_attributes(tag_attributes)
            .clean_content_tags(["script", "style"].into_iter().collect())
            .url_schemes(["https", "http", "mailto"].into_iter().collect())
            .url_relative(UrlRelative::Deny)
            .link_rel(Some("nofollow noopener noreferrer ugc"))
            .set_tag_attribute_value("a", "target", "_blank")
            .attribute_filter(|element, attribute, value| match (element, attribute) {
                ("img", "src") if !value.trim_start().to_ascii_lowercase().starts_with("https://") => None,
                _ => Some(Cow::Borrowed(value)),
            });
        Self { inner: Arc::new(Inner { sanitizer, cache: Mutex::new(RenderCache::default()), permits: Arc::new(Semaphore::new(CONCURRENT_RENDERS)), timeout: RENDER_TIMEOUT }) }
    }
}

impl Inner {
    fn convert(&self, markdown: &str, deadline: Instant) -> Conversion {
        let mut events = Vec::new();
        let (mut depth, mut elements, mut raw_tags) = (0usize, 0usize, 0usize);
        for event in Parser::new_ext(markdown, Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH) {
            match &event {
                Event::Start(_) => {
                    depth += 1;
                    elements += 1;
                    if depth > MAX_NESTING_DEPTH || elements > MAX_ELEMENTS {
                        return Conversion::TooComplex;
                    }
                }
                Event::End(_) => depth = depth.saturating_sub(1),
                Event::Html(raw) | Event::InlineHtml(raw) => {
                    raw_tags += raw.bytes().filter(|b| *b == b'<').count();
                    if raw_tags > MAX_RAW_HTML_TAGS {
                        return Conversion::TooComplex;
                    }
                }
                _ => {}
            }
            if events.len() % 256 == 0 && Instant::now() >= deadline {
                return Conversion::TimedOut;
            }
            events.push(event);
        }
        let mut html = String::new();
        html::push_html(&mut html, events.into_iter());
        Conversion::Html(self.sanitizer.clean(&html).to_string())
    }
}

impl Default for PulldownAmmoniaReadmeRenderer {
    fn default() -> Self {
        Self::new()
    }
}

enum Conversion {
    Html(String),
    TooComplex,
    TimedOut,
}

#[async_trait]
impl ReadmeRendererPort for PulldownAmmoniaReadmeRenderer {
    async fn render(&self, markdown: &str) -> String {
        let markdown = truncate_at_char_boundary(markdown, MAX_README_BYTES);
        let key: [u8; 32] = Sha256::digest(markdown.as_bytes()).into();
        let inner = &self.inner;
        if let Some(html) = inner.cache.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
            return html;
        }
        let deadline = Instant::now() + inner.timeout;
        let Ok(Ok(permit)) = tokio::time::timeout(inner.timeout, inner.permits.clone().acquire_owned()).await else {
            return plain_text(markdown);
        };
        let (worker, owned) = (inner.clone(), markdown.to_string());
        let task = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            worker.convert(&owned, deadline)
        });
        let html = match tokio::time::timeout(inner.timeout, task).await {
            Ok(Ok(Conversion::Html(html))) => html,
            Ok(Ok(Conversion::TooComplex)) => plain_text(markdown),
            _ => return plain_text(markdown),
        };
        inner.cache.lock().unwrap_or_else(|p| p.into_inner()).insert(key, html.clone());
        html
    }
}

/// The README as inert text: every character that could start markup is escaped.
fn plain_text(markdown: &str) -> String {
    let mut html = String::with_capacity(markdown.len() + 16);
    html.push_str("<pre>");
    for c in markdown.chars() {
        match c {
            '&' => html.push_str("&amp;"),
            '<' => html.push_str("&lt;"),
            '>' => html.push_str("&gt;"),
            '"' => html.push_str("&quot;"),
            '\'' => html.push_str("&#39;"),
            _ => html.push(c),
        }
    }
    html.push_str("</pre>");
    html
}

fn truncate_at_char_boundary(text: &str, max_bytes: usize) -> &str {
    if text.len() <= max_bytes {
        return text;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

/// Bounded by entry count and by total size; the oldest render goes first.
#[derive(Default)]
struct RenderCache {
    entries: HashMap<[u8; 32], Arc<str>>,
    order: VecDeque<[u8; 32]>,
    bytes: usize,
}

impl RenderCache {
    fn get(&self, key: &[u8; 32]) -> Option<String> {
        self.entries.get(key).map(|html| html.to_string())
    }

    fn insert(&mut self, key: [u8; 32], html: String) {
        if html.len() > CACHE_MAX_BYTES || self.entries.contains_key(&key) {
            return;
        }
        while self.entries.len() >= CACHE_MAX_ENTRIES || self.bytes + html.len() > CACHE_MAX_BYTES {
            let Some(oldest) = self.order.pop_front() else { break };
            if let Some(evicted) = self.entries.remove(&oldest) {
                self.bytes -= evicted.len();
            }
        }
        self.bytes += html.len();
        self.order.push_back(key);
        self.entries.insert(key, html.into());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(markdown: &str) -> String {
        let markdown = truncate_at_char_boundary(markdown, MAX_README_BYTES);
        match PulldownAmmoniaReadmeRenderer::new().inner.convert(markdown, Instant::now() + Duration::from_secs(60)) {
            Conversion::Html(html) => html,
            _ => panic!("expected {markdown:.60} to convert"),
        }
    }

    fn adversarial_corpus() -> Vec<(&'static str, String)> {
        let table_rows = "|1|2|\n".repeat(30_000);
        vec![
            ("nested blockquotes", "> ".repeat(250_000)),
            ("nested bullet lists", "* ".repeat(250_000)),
            ("nested ordered lists", "1. ".repeat(100_000)),
            ("indented nested lists", (0..2_000).map(|depth| format!("{}- x\n", " ".repeat(depth))).collect()),
            ("unclosed divs", "<div>".repeat(100_000)),
            ("unclosed inline html", "<b><i><a href=\"https://e.x\">".repeat(30_000)),
            ("unmatched brackets", "[".repeat(200_000)),
            ("unmatched emphasis", "*a ".repeat(170_000)),
            ("deep emphasis", "*_**__".repeat(40_000)),
            ("long table", format!("|a|b|\n|-|-|\n{table_rows}")),
            ("wide table", format!("{}|\n{}|\n", "|a".repeat(60_000), "|-".repeat(60_000))),
            ("link reference definitions", "[a]: http://example.com\n".repeat(20_000)),
            ("references used many times", format!("[a]: http://example.com\n{}", "[a] ".repeat(60_000))),
            ("nested images and links", "[![a](https://e.x/i.png)](".repeat(30_000)),
            ("entities", "&amp;&lt;&#x41;".repeat(40_000)),
            ("backticks", "`a".repeat(100_000)),
            ("just under the element budget", "`a` ".repeat(19_000)),
            ("just under the raw html budget", "<div><i>x</i></div>".repeat(300)),
            ("unclosed inline html at the budget", "<i>x".repeat(1_990)),
            ("deep but allowed", format!("{}x", "> ".repeat(30))),
        ]
    }

    fn convert_now(markdown: &str) -> (Conversion, Duration) {
        let markdown = truncate_at_char_boundary(markdown, MAX_README_BYTES);
        let started = Instant::now();
        let outcome = PulldownAmmoniaReadmeRenderer::new().inner.convert(markdown, started + Duration::from_secs(60));
        (outcome, started.elapsed())
    }

    fn renderer_with(timeout: Duration) -> PulldownAmmoniaReadmeRenderer {
        let renderer = PulldownAmmoniaReadmeRenderer::new();
        let inner = Arc::try_unwrap(renderer.inner).ok().unwrap();
        PulldownAmmoniaReadmeRenderer { inner: Arc::new(Inner { timeout, ..inner }) }
    }

    #[test]
    fn every_hostile_readme_is_converted_or_refused_within_two_seconds_even_in_a_debug_build() {
        for (name, markdown) in adversarial_corpus() {
            let (outcome, took) = convert_now(&markdown);

            assert!(took < Duration::from_secs(2), "{name} took {took:?}");
            if let Conversion::Html(html) = outcome {
                assert!(!html.contains("<div") && !html.contains("<script"), "{name}: {html:.100}");
            }
        }
    }

    #[test]
    fn deep_nesting_and_a_flood_of_elements_or_raw_tags_are_refused_rather_than_converted() {
        for (name, markdown) in [
            ("blockquotes", "> ".repeat(250_000)),
            ("ordered lists", "1. ".repeat(100_000)),
            ("indented lists", (0..2_000).map(|depth| format!("{}- x\n", " ".repeat(depth))).collect()),
            ("unclosed divs", "<div>".repeat(100_000)),
            ("unclosed inline html", "<b><i><a href=\"https://e.x\">".repeat(30_000)),
            ("table rows", format!("|a|b|\n|-|-|\n{}", "|1|2|\n".repeat(30_000))),
            ("emphasis spans", "*a* ".repeat(30_000)),
        ] {
            assert!(matches!(convert_now(&markdown).0, Conversion::TooComplex), "{name}");
        }
    }

    #[test]
    fn realistic_nesting_is_still_converted() {
        let (outcome, _) = convert_now("> quote\n>\n> - item\n>   - nested **bold [link](https://example.com)**\n>     1. deeper\n\n<div align=\"center\"><img src=\"https://example.com/a.png\"></div>");

        let Conversion::Html(html) = outcome else { panic!("refused") };
        assert!(html.contains("<blockquote>") && html.contains("<strong>") && html.contains("<img"), "{html}");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_readme_is_shown_as_escaped_text_and_never_as_markup() {
        let markdown = format!("<script>alert(1)</script> & \"quoted\" 'x'\n{}", "> ".repeat(100_000));

        let html = PulldownAmmoniaReadmeRenderer::new().render(&markdown).await;

        assert!(html.starts_with("<pre>&lt;script&gt;alert(1)&lt;/script&gt; &amp; &quot;quoted&quot; &#39;x&#39;\n&gt; "), "{html:.120}");
        assert!(html.ends_with("</pre>"));
        assert!(!html.contains("<script") && !html.contains("<blockquote"));
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn hostile_readmes_come_back_quickly_through_the_async_port() {
        let renderer = PulldownAmmoniaReadmeRenderer::new();
        for (name, markdown) in adversarial_corpus() {
            let started = Instant::now();

            let html = renderer.render(&markdown).await;

            assert!(started.elapsed() < Duration::from_secs(2), "{name} took {:?}", started.elapsed());
            assert!(!html.contains("<script") && !html.contains("<div"), "{name}");
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn converting_does_not_block_the_async_runtime() {
        let renderer = PulldownAmmoniaReadmeRenderer::new();
        let markdown = "`a` ".repeat(10_000);
        let mut polls = 0usize;

        let html = tokio::select! {
            html = renderer.render(&markdown) => html,
            _ = async { loop { polls += 1; tokio::task::yield_now().await } } => unreachable!(),
        };

        assert!(!html.is_empty());
        assert!(polls > 0, "the runtime kept running while the render was in flight");
    }

    #[tokio::test]
    async fn a_render_that_runs_out_of_time_falls_back_to_text_and_is_not_remembered() {
        let renderer = renderer_with(Duration::ZERO);

        let html = renderer.render("# Title").await;

        assert_eq!(html, "<pre># Title</pre>");
        assert!(renderer.inner.cache.lock().unwrap().entries.is_empty());
    }

    #[tokio::test]
    async fn when_every_render_slot_is_busy_the_readme_is_shown_as_text_after_the_timeout() {
        let renderer = renderer_with(Duration::from_millis(50));
        let _busy = renderer.inner.permits.clone().acquire_many_owned(CONCURRENT_RENDERS as u32).await.unwrap();

        let started = Instant::now();
        let html = renderer.render("# Title").await;

        assert_eq!(html, "<pre># Title</pre>");
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[tokio::test]
    async fn a_repeated_readme_is_served_from_the_cache() {
        let renderer = PulldownAmmoniaReadmeRenderer::new();

        let first = renderer.render("# Title\n\ntext").await;
        let second = renderer.render("# Title\n\ntext").await;

        assert_eq!(first, second);
        assert!(first.contains("<h1>Title</h1>"));
        assert_eq!(renderer.inner.cache.lock().unwrap().entries.len(), 1);
    }

    #[test]
    fn the_cache_stays_within_its_entry_and_size_bounds_and_drops_the_oldest_first() {
        let mut cache = RenderCache::default();
        for i in 0..CACHE_MAX_ENTRIES + 10 {
            cache.insert(Sha256::digest(i.to_string()).into(), format!("<p>{i}</p>"));
        }

        assert_eq!(cache.entries.len(), CACHE_MAX_ENTRIES);
        assert!(cache.get(&Sha256::digest("0").into()).is_none() && cache.get(&Sha256::digest((CACHE_MAX_ENTRIES + 9).to_string()).into()).is_some());

        let mut cache = RenderCache::default();
        let big = "x".repeat(CACHE_MAX_BYTES / 3 + 1);
        for i in 0..4 {
            cache.insert(Sha256::digest(i.to_string()).into(), big.clone());
        }
        assert!(cache.bytes <= CACHE_MAX_BYTES && cache.entries.len() == 2, "{} entries, {} bytes", cache.entries.len(), cache.bytes);
    }

    #[test]
    fn renders_common_markdown() {
        let html = render("# Title\n\nSome **bold**, _italic_ and `code`.\n\n- one\n- two\n\n```\nlet x = 1;\n```\n\n> quoted\n\n~~gone~~");

        for expected in ["<h1>Title</h1>", "<strong>bold</strong>", "<em>italic</em>", "<code>code</code>", "<ul>", "<li>one</li>", "<pre><code>let x = 1;", "<blockquote>", "<del>gone</del>"] {
            assert!(html.contains(expected), "missing {expected} in {html}");
        }
    }

    #[test]
    fn renders_tables() {
        let html = render("| a | b |\n|---|---|\n| 1 | 2 |\n");

        assert!(html.contains("<table>") && html.contains("<th>a</th>") && html.contains("<td>2</td>"), "{html}");
    }

    #[test]
    fn links_open_in_a_new_tab_and_are_not_endorsed() {
        let html = render("[docs](https://example.com/docs)");

        assert!(html.contains(r#"href="https://example.com/docs""#), "{html}");
        assert!(html.contains(r#"target="_blank""#), "{html}");
        assert!(html.contains(r#"rel="nofollow noopener noreferrer ugc""#), "{html}");
    }

    #[test]
    fn a_link_cannot_override_its_own_target_or_rel() {
        let html = render(r#"<a href="https://example.com" target="_self" rel="opener">x</a>"#);

        assert!(html.contains(r#"target="_blank""#) && !html.contains("_self"), "{html}");
        assert!(html.contains(r#"rel="nofollow noopener noreferrer ugc""#) && !html.contains(r#"rel="opener""#), "{html}");
    }

    #[test]
    fn scripts_and_their_content_are_removed() {
        let html = render("before\n\n<script>alert(1)</script>\n\nafter");

        assert!(!html.contains("script") && !html.contains("alert"), "{html}");
        assert!(html.contains("before") && html.contains("after"));
    }

    #[test]
    fn event_handlers_and_inline_styles_are_removed() {
        let html = render(r#"<p onclick="steal()" style="position:fixed;inset:0" class="x" id="y">hi</p><img src="https://example.com/a.png" onerror="steal()" style="width:9999px">"#);

        for forbidden in ["onclick", "onerror", "style", "class=", "id=", "steal"] {
            assert!(!html.contains(forbidden), "{forbidden} survived in {html}");
        }
        assert!(html.contains("<p>hi</p>"));
    }

    #[test]
    fn frames_forms_objects_and_svg_are_removed() {
        let html = render(
            r#"<iframe src="https://evil.example"></iframe><form action="https://evil.example"><input name="pw"><button>go</button></form><object data="x"></object><embed src="x"><svg onload="steal()"><circle/></svg><meta http-equiv="refresh" content="0;url=https://evil.example"><link rel="stylesheet" href="https://evil.example/x.css"><base href="https://evil.example/">"#,
        );

        for forbidden in ["iframe", "<form", "<input", "<button", "<object", "<embed", "<svg", "<meta", "<link", "<base", "steal"] {
            assert!(!html.contains(forbidden), "{forbidden} survived in {html}");
        }
    }

    #[test]
    fn dangerous_link_schemes_are_dropped_whatever_their_spelling() {
        for link in [
            "[x](javascript:alert(1))",
            "[x](JaVaScRiPt:alert(1))",
            "[x](&#106;avascript:alert(1))",
            "[x](data:text/html;base64,PHNjcmlwdD4=)",
            "[x](vbscript:msgbox)",
            "[x](file:///etc/passwd)",
            r#"<a href="  javascript:alert(1)">x</a>"#,
            r#"<a href="java&#x09;script:alert(1)">x</a>"#,
        ] {
            let html = render(link);
            assert!(!html.to_ascii_lowercase().contains("javascript") && !html.contains("data:") && !html.contains("vbscript") && !html.contains("file:"), "{link} -> {html}");
        }
    }

    #[test]
    fn relative_links_are_dropped_because_they_would_resolve_against_the_app() {
        let html = render("[a](./docs/guide.md) [b](/etc/passwd) [c](#section) [d](//evil.example/x)");

        assert!(!html.contains("href"), "{html}");
        assert!(html.contains('a') && html.contains('d'), "the text stays");
    }

    #[test]
    fn mailto_and_plain_http_links_are_kept() {
        let html = render("[mail](mailto:dev@example.com) [site](http://example.com)");

        assert!(html.contains(r#"href="mailto:dev@example.com""#) && html.contains(r#"href="http://example.com""#), "{html}");
    }

    #[test]
    fn only_https_images_are_kept() {
        let html = render(
            "![ok](https://example.com/a.png) ![plain](http://example.com/b.png) ![data](data:image/png;base64,AAAA) ![rel](./c.png) ![js](javascript:alert(1)) <img src=\"  HTTPS://example.com/d.png\">",
        );

        assert!(html.contains(r#"src="https://example.com/a.png""#), "{html}");
        assert!(html.to_ascii_lowercase().contains("https://example.com/d.png"), "{html}");
        for forbidden in ["http://example.com/b.png", "data:", "./c.png", "javascript"] {
            assert!(!html.contains(forbidden), "{forbidden} survived in {html}");
        }
    }

    #[test]
    fn image_attributes_beyond_the_allowlist_are_removed() {
        let html = render(r#"<img src="https://example.com/a.png" srcset="https://example.com/b.png 2x" width="9999" loading="eager" alt="pic" title="t">"#);

        assert!(html.contains(r#"alt="pic""#) && html.contains(r#"title="t""#), "{html}");
        for forbidden in ["srcset", "width", "loading"] {
            assert!(!html.contains(forbidden), "{forbidden} survived in {html}");
        }
    }

    #[test]
    fn html_comments_and_cdata_do_not_smuggle_content() {
        let html = render("a <!-- <script>alert(1)</script> --> b <![CDATA[<script>alert(2)</script>]]> c");

        // A CDATA section is a bogus comment in HTML, so a little of it can remain as text; what matters is that no markup does.
        assert!(!html.contains("<script") && !html.contains("<!--") && !html.contains("alert(1)"), "{html}");
        assert_eq!(html.matches('<').count(), 2, "only the paragraph's own tags remain: {html}");
    }

    #[test]
    fn broken_and_nested_markup_still_comes_out_well_formed() {
        let html = render("<p><b>unclosed <div><script>x</script></p><table><tr><td><a href=\"https://example.com\">l");

        assert!(!html.contains("script"), "{html}");
        assert!(html.matches("<a ").count() == html.matches("</a>").count(), "{html}");
    }

    #[test]
    fn details_and_summary_survive() {
        let html = render("<details><summary>More</summary>\n\nhidden text\n\n</details>");

        assert!(html.contains("<details>") && html.contains("<summary>More</summary>") && html.contains("hidden text"), "{html}");
    }

    #[test]
    fn oversized_input_is_cut_at_the_limit_without_splitting_a_character() {
        let readme = "é".repeat(MAX_README_BYTES);

        let html = render(&readme);

        assert!(html.len() <= MAX_README_BYTES + 16, "the output stays bounded, got {}", html.len());
        assert!(html.starts_with("<p>é"));
    }

    #[test]
    fn an_empty_readme_renders_to_nothing() {
        assert_eq!(render(""), "");
    }
}
