use async_trait::async_trait;

/// Longest README the catalog will render. Anything beyond this is cut off before conversion.
pub const MAX_README_BYTES: usize = 128 * 1024;

/// Turns a package's Markdown README into HTML that is safe to embed in a page. Implementations must
/// treat the input as hostile: whatever comes back may be inserted into the DOM as is. It never fails and
/// never takes unbounded time: input it cannot convert cheaply comes back as plain text.
#[async_trait]
pub trait ReadmeRendererPort: Send + Sync {
    async fn render(&self, markdown: &str) -> String;
}
