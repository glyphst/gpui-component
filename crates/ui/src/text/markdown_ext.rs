use std::{
    any::Any,
    collections::HashMap,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use gpui::{AnyElement, App, IntoElement, SharedString, TextStyle, Window};
use markdown::{ParseOptions, mdast};

use crate::text::node::Span;

static MARKDOWN_EXTENSIONS_REVISION: AtomicU64 = AtomicU64::new(1);

/// Re-export of the Markdown AST types used by custom parsers.
pub use markdown::mdast as markdown_ast;

/// Type for a custom Markdown block parser.
///
/// Parsers run during Markdown AST conversion, often on a background task. They
/// must not depend on [`Window`] or [`App`]; return parsed, reusable data in a
/// [`MarkdownNode`] and render it later with a block renderer.
pub type MarkdownBlockParserFn =
    dyn for<'a> Fn(&mdast::Node, &MarkdownParseContext<'a>) -> Option<MarkdownNode> + Send + Sync;

/// Type for a custom Markdown block renderer.
pub type MarkdownBlockRenderFn =
    dyn Fn(&MarkdownNode, &mut Window, &mut App) -> AnyElement + Send + Sync;

/// Type for a custom Markdown phrasing-node parser.
pub type MarkdownInlineParserFn =
    dyn for<'a> Fn(&mdast::Node, &MarkdownParseContext<'a>) -> Option<MarkdownNode> + Send + Sync;

/// Type for a custom Markdown phrasing-node renderer.
///
/// The inherited [`TextStyle`] is the style active at the node's actual layout
/// position, including heading sizes and surrounding text color.
pub type MarkdownInlineRenderFn =
    dyn Fn(&MarkdownNode, &TextStyle, &mut Window, &mut App) -> AnyElement + Send + Sync;

/// A reusable Markdown extension that parses and renders one custom node.
pub trait MarkdownPlugin: Send + Sync + 'static {
    /// Whether this plugin produces block-level nodes.
    ///
    /// Plugins are inline by default. Block plugins should return `true`.
    fn is_block(&self) -> bool {
        false
    }

    /// Stable name for nodes produced by this plugin.
    fn name(&self) -> &str;

    /// Convert an mdast node into a custom Markdown node.
    fn parse(&self, node: &mdast::Node, cx: &MarkdownParseContext<'_>) -> Option<MarkdownNode>;

    /// Render a custom Markdown node produced by this plugin.
    fn render(&self, node: &MarkdownNode, window: &mut Window, cx: &mut App) -> impl IntoElement;

    /// Render an inline custom Markdown node with its inherited text style.
    ///
    /// Plugins that do not need inherited styling can rely on this default,
    /// which delegates to [`Self::render`].
    fn render_inline(
        &self,
        node: &MarkdownNode,
        _text_style: &TextStyle,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        self.render(node, window, cx).into_any_element()
    }
}

/// Context passed to custom Markdown parsers.
pub struct MarkdownParseContext<'a> {
    source: &'a str,
    offset: usize,
}

impl<'a> MarkdownParseContext<'a> {
    pub(crate) fn new(source: &'a str, offset: usize) -> Self {
        Self { source, offset }
    }

    /// Source text for the Markdown fragment currently being parsed.
    pub fn source(&self) -> &'a str {
        self.source
    }

    /// Byte offset of `source` in the full document when parsing an appended
    /// fragment.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Source slice for a specific mdast node.
    pub fn node_source(&self, node: &mdast::Node) -> Option<&'a str> {
        let position = node.position()?;
        self.source.get(position.start.offset..position.end.offset)
    }
}

/// A custom Markdown node produced by [`MarkdownExtensions`].
#[derive(Clone)]
pub struct MarkdownNode {
    name: SharedString,
    text: SharedString,
    markdown: SharedString,
    data: Arc<dyn Any + Send + Sync>,
    layout: MarkdownNodeLayout,
    pub(crate) span: Option<Span>,
}

/// Layout requested by a custom Markdown phrasing node.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum MarkdownNodeLayout {
    /// Participate in the paragraph's inline wrapping flow.
    #[default]
    Inline,
    /// Occupy the full document width when it is the paragraph's sole child.
    FullWidth,
}

impl MarkdownNode {
    /// Create a custom Markdown node with a stable name and typed data.
    pub fn new<T>(name: impl Into<SharedString>, data: T) -> Self
    where
        T: Any + Send + Sync + 'static,
    {
        Self {
            name: name.into(),
            text: SharedString::default(),
            markdown: SharedString::default(),
            data: Arc::new(data),
            layout: MarkdownNodeLayout::Inline,
            span: None,
        }
    }

    /// Stable name for this custom node.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Text representation of this custom node.
    pub fn as_text(&self) -> &str {
        &self.text
    }

    /// Markdown representation of this custom node.
    pub fn as_markdown(&self) -> &str {
        &self.markdown
    }

    /// Set the text representation of this custom node.
    pub fn text(mut self, text: impl Into<SharedString>) -> Self {
        self.text = text.into();
        self
    }

    /// Set the Markdown representation of this custom node.
    pub fn markdown(mut self, markdown: impl Into<SharedString>) -> Self {
        self.markdown = markdown.into();
        self
    }

    /// Request full-width layout when this phrasing node is the paragraph's
    /// sole child.
    pub fn full_width(mut self) -> Self {
        self.layout = MarkdownNodeLayout::FullWidth;
        self
    }

    /// Read typed data.
    pub fn data<T>(&self) -> Option<&T>
    where
        T: Any + Send + Sync + 'static,
    {
        self.data.downcast_ref()
    }

    pub(crate) fn set_span(&mut self, span: Option<Span>) {
        self.span = span;
    }

    pub(crate) fn layout(&self) -> MarkdownNodeLayout {
        self.layout
    }

    pub(crate) fn to_markdown(&self) -> String {
        if self.markdown.is_empty() {
            self.text.to_string()
        } else {
            self.markdown.to_string()
        }
    }
}

impl fmt::Debug for MarkdownNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MarkdownNode")
            .field("name", &self.name)
            .field("text", &self.text)
            .field("markdown", &self.markdown)
            .field("layout", &self.layout)
            .field("span", &self.span)
            .finish_non_exhaustive()
    }
}

impl PartialEq for MarkdownNode {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
            && self.text == other.text
            && self.markdown == other.markdown
            && self.layout == other.layout
            && self.span == other.span
    }
}

/// Registry for custom Markdown parsing and rendering.
#[derive(Clone, Default)]
pub struct MarkdownExtensions {
    enable_mdx: bool,
    enable_math: bool,
    block_parsers: Vec<Arc<MarkdownBlockParserFn>>,
    block_renderers: HashMap<SharedString, Arc<MarkdownBlockRenderFn>>,
    inline_parsers: Vec<Arc<MarkdownInlineParserFn>>,
    inline_renderers: HashMap<SharedString, Arc<MarkdownInlineRenderFn>>,
    revision: u64,
}

impl MarkdownExtensions {
    /// Enable MDX JSX/expression constructs.
    ///
    /// This disables raw HTML constructs because `markdown-rs` gives HTML
    /// priority over MDX when both are enabled.
    pub fn mdx(mut self) -> Self {
        self.enable_mdx = true;
        self.bump_revision();
        self
    }

    /// Enable `$...$` and `$$...$$` parsing in `markdown-rs`.
    pub fn math(mut self) -> Self {
        self.enable_math = true;
        self.bump_revision();
        self
    }

    /// Register a parser for block-level Markdown AST nodes.
    pub fn block_parser<F>(mut self, parser: F) -> Self
    where
        F: for<'a> Fn(&mdast::Node, &MarkdownParseContext<'a>) -> Option<MarkdownNode>
            + Send
            + Sync
            + 'static,
    {
        self.push_block_parser(parser);
        self
    }

    /// Register a renderer for a custom block node name.
    pub fn block_renderer<F, E>(mut self, name: impl Into<SharedString>, renderer: F) -> Self
    where
        F: Fn(&MarkdownNode, &mut Window, &mut App) -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        self.push_block_renderer(name, renderer);
        self
    }

    /// Register a parser for Markdown phrasing nodes.
    pub fn inline_parser<F>(mut self, parser: F) -> Self
    where
        F: for<'a> Fn(&mdast::Node, &MarkdownParseContext<'a>) -> Option<MarkdownNode>
            + Send
            + Sync
            + 'static,
    {
        self.push_inline_parser(parser);
        self
    }

    /// Register a renderer for a custom Markdown phrasing node name.
    pub fn inline_renderer<F, E>(mut self, name: impl Into<SharedString>, renderer: F) -> Self
    where
        F: Fn(&MarkdownNode, &TextStyle, &mut Window, &mut App) -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        self.push_inline_renderer(name, renderer);
        self
    }

    /// Apply a reusable Markdown plugin.
    pub fn plugin<P>(self, plugin: P) -> Self
    where
        P: MarkdownPlugin,
    {
        let plugin = Arc::new(plugin);
        let name = SharedString::from(plugin.name().to_string());
        let parser = plugin.clone();
        let renderer = plugin;

        if parser.is_block() {
            let mut extensions = self.block_parser(move |node, cx| parser.parse(node, cx));
            extensions.push_block_renderer(name, move |node, window, cx| {
                renderer.render(node, window, cx).into_any_element()
            });
            extensions
        } else {
            let mut extensions = self.inline_parser(move |node, cx| parser.parse(node, cx));
            extensions.push_inline_renderer(name, move |node, text_style, window, cx| {
                renderer.render_inline(node, text_style, window, cx)
            });
            extensions
        }
    }

    pub(crate) fn revision(&self) -> u64 {
        self.revision
    }

    pub(crate) fn push_block_parser<F>(&mut self, parser: F)
    where
        F: for<'a> Fn(&mdast::Node, &MarkdownParseContext<'a>) -> Option<MarkdownNode>
            + Send
            + Sync
            + 'static,
    {
        self.block_parsers.push(Arc::new(parser));
        self.bump_revision();
    }

    pub(crate) fn push_block_renderer<F, E>(&mut self, name: impl Into<SharedString>, renderer: F)
    where
        F: Fn(&MarkdownNode, &mut Window, &mut App) -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        self.block_renderers.insert(
            name.into(),
            Arc::new(move |node, window, cx| renderer(node, window, cx).into_any_element()),
        );
        self.bump_revision();
    }

    pub(crate) fn push_inline_parser<F>(&mut self, parser: F)
    where
        F: for<'a> Fn(&mdast::Node, &MarkdownParseContext<'a>) -> Option<MarkdownNode>
            + Send
            + Sync
            + 'static,
    {
        self.inline_parsers.push(Arc::new(parser));
        self.bump_revision();
    }

    pub(crate) fn push_inline_renderer<F, E>(&mut self, name: impl Into<SharedString>, renderer: F)
    where
        F: Fn(&MarkdownNode, &TextStyle, &mut Window, &mut App) -> E + Send + Sync + 'static,
        E: IntoElement,
    {
        self.inline_renderers.insert(
            name.into(),
            Arc::new(move |node, text_style, window, cx| {
                renderer(node, text_style, window, cx).into_any_element()
            }),
        );
        self.bump_revision();
    }

    pub(crate) fn parse_options(&self) -> ParseOptions {
        let mut options = ParseOptions::gfm();
        if self.enable_mdx {
            options.constructs.html_flow = false;
            options.constructs.html_text = false;
            options.constructs.mdx_expression_flow = true;
            options.constructs.mdx_expression_text = true;
            options.constructs.mdx_jsx_flow = true;
            options.constructs.mdx_jsx_text = true;
        }
        if self.enable_math {
            options.constructs.math_flow = true;
            options.constructs.math_text = true;
        }
        options
    }

    pub(crate) fn parse_block(
        &self,
        node: &mdast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        for parser in &self.block_parsers {
            if let Some(node) = parser(node, cx) {
                return Some(node);
            }
        }
        None
    }

    pub(crate) fn render_block(
        &self,
        node: &MarkdownNode,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.block_renderers
            .get(node.name())
            .map(|render| render(node, window, cx))
    }

    pub(crate) fn parse_inline(
        &self,
        node: &mdast::Node,
        cx: &MarkdownParseContext<'_>,
    ) -> Option<MarkdownNode> {
        for parser in &self.inline_parsers {
            if let Some(node) = parser(node, cx) {
                return Some(node);
            }
        }
        None
    }

    pub(crate) fn render_inline(
        &self,
        node: &MarkdownNode,
        text_style: &TextStyle,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyElement> {
        self.inline_renderers
            .get(node.name())
            .map(|render| render(node, text_style, window, cx))
    }

    fn bump_revision(&mut self) {
        self.revision = MARKDOWN_EXTENSIONS_REVISION.fetch_add(1, Ordering::Relaxed);
    }
}
