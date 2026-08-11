use std::{
    any::Any,
    collections::HashMap,
    fmt,
    ops::Range,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use gpui::{AnyElement, App, IntoElement, SharedString, TextStyle, Window};
use markdown::{ParseOptions, mdast};

use crate::text::node::Span;

static MARKDOWN_EXTENSIONS_REVISION: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LatexMathMode {
    Inline,
    Display,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct LatexMathCandidate {
    range: Range<usize>,
    mode: LatexMathMode,
}

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

    /// Enable `$...$`, `$$...$$`, `\(...\)`, and `\[...\]` math parsing.
    pub fn math(mut self) -> Self {
        self.enable_math = true;
        self.bump_revision();
        self
    }

    /// Parse Markdown with all constructs enabled on these extensions.
    ///
    /// LaTeX-style math delimiters are converted only in a temporary,
    /// byte-for-byte parsing copy. Node positions therefore continue to refer
    /// to `source`, so selection and source reconstruction preserve the exact
    /// delimiters supplied by the caller.
    pub fn parse_ast(&self, source: &str) -> Result<mdast::Node, SharedString> {
        let options = self.parse_options();
        let original = markdown::to_mdast(source, &options)
            .map_err(|error| SharedString::from(error.to_string()))?;
        if !self.enable_math {
            return Ok(original);
        }

        let mut candidates = latex_math_candidates(source, &original);
        if candidates.is_empty() {
            return Ok(original);
        }

        loop {
            let normalized = normalize_latex_math_candidates(source, &candidates);
            let Ok(parsed) = markdown::to_mdast(&normalized, &options) else {
                return Ok(original);
            };
            let parsed_math = parsed_math_ranges(&parsed);
            let retained = candidates
                .iter()
                .filter(|candidate| {
                    parsed_math.iter().any(|(range, is_display)| {
                        range == &candidate.range
                            && (candidate.mode == LatexMathMode::Display || !is_display)
                    })
                })
                .cloned()
                .collect::<Vec<_>>();

            if retained.len() == candidates.len() {
                return Ok(parsed);
            }
            if retained.is_empty() {
                return Ok(original);
            }
            candidates = retained;
        }
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

fn latex_math_candidates(source: &str, root: &mdast::Node) -> Vec<LatexMathCandidate> {
    if !source.contains("\\(") && !source.contains("\\[") {
        return Vec::new();
    }

    let mut containers = Vec::new();
    let mut protected = Vec::new();
    collect_math_source_ranges(root, &mut containers, &mut protected);
    protected.sort_by_key(|range| range.start);

    let mut candidates = Vec::new();
    collect_display_candidates(source, &protected, &mut candidates);

    let mut barriers = protected.clone();
    barriers.extend(candidates.iter().map(|candidate| candidate.range.clone()));
    barriers.sort_by_key(|range| range.start);

    for container in containers {
        if let Some(display) = standalone_display_candidate(source, &container, &protected) {
            candidates.push(display);
            continue;
        }

        let mut cursor = container.start;
        for barrier in barriers
            .iter()
            .filter(|barrier| barrier.start < container.end && barrier.end > container.start)
        {
            let barrier_start = barrier.start.clamp(container.start, container.end);
            if cursor < barrier_start {
                collect_inline_candidates(source, cursor..barrier_start, &mut candidates);
            }
            cursor = cursor.max(barrier.end.min(container.end));
        }
        if cursor < container.end {
            collect_inline_candidates(source, cursor..container.end, &mut candidates);
        }
    }

    candidates.sort_by_key(|candidate| candidate.range.start);
    candidates.dedup_by(|left, right| left.range == right.range);
    candidates
}

fn collect_math_source_ranges(
    node: &mdast::Node,
    containers: &mut Vec<Range<usize>>,
    protected: &mut Vec<Range<usize>>,
) {
    let range = node
        .position()
        .map(|position| position.start.offset..position.end.offset);

    match node {
        mdast::Node::Paragraph(_) | mdast::Node::Heading(_) | mdast::Node::TableCell(_) => {
            if let Some(range) = range.clone() {
                containers.push(range);
            }
        }
        mdast::Node::InlineCode(_)
        | mdast::Node::InlineMath(_)
        | mdast::Node::Code(_)
        | mdast::Node::Math(_)
        | mdast::Node::Html(_)
        | mdast::Node::Link(_)
        | mdast::Node::LinkReference(_)
        | mdast::Node::Image(_)
        | mdast::Node::ImageReference(_)
        | mdast::Node::Definition(_)
        | mdast::Node::MdxjsEsm(_)
        | mdast::Node::MdxTextExpression(_)
        | mdast::Node::MdxFlowExpression(_)
        | mdast::Node::MdxJsxTextElement(_)
        | mdast::Node::MdxJsxFlowElement(_)
        | mdast::Node::Toml(_)
        | mdast::Node::Yaml(_) => {
            if let Some(range) = range {
                protected.push(range);
            }
            return;
        }
        _ => {}
    }

    if let Some(children) = node.children() {
        for child in children {
            collect_math_source_ranges(child, containers, protected);
        }
    }
}

fn standalone_display_candidate(
    source: &str,
    container: &Range<usize>,
    protected: &[Range<usize>],
) -> Option<LatexMathCandidate> {
    let container_source = source.get(container.clone())?;
    let trimmed_start = container_source.len() - container_source.trim_start().len();
    let trimmed_end = container_source.trim_end().len();
    let start = container.start + trimmed_start;
    let end = container.start + trimmed_end;
    let candidate_source = source.get(start..end)?;

    if !candidate_source.starts_with("\\[")
        || !candidate_source.ends_with("\\]")
        || candidate_source.len() <= 4
        || !is_unescaped_backslash(source.as_bytes(), start)
        || !is_unescaped_backslash(source.as_bytes(), end - 2)
        || protected
            .iter()
            .any(|barrier| barrier.start < end && barrier.end > start)
        || !safe_dollar_fence_contents(source.get(start + 2..end - 2)?)
    {
        return None;
    }

    Some(LatexMathCandidate {
        range: start..end,
        mode: LatexMathMode::Display,
    })
}

fn collect_display_candidates(
    source: &str,
    protected: &[Range<usize>],
    candidates: &mut Vec<LatexMathCandidate>,
) {
    let bytes = source.as_bytes();
    let mut cursor = 0;

    while cursor + 1 < bytes.len() {
        if bytes[cursor] != b'\\'
            || bytes[cursor + 1] != b'['
            || !is_unescaped_backslash(bytes, cursor)
            || !line_prefix_is_whitespace(source, cursor)
            || protected
                .iter()
                .any(|barrier| barrier.start <= cursor && cursor < barrier.end)
        {
            cursor += 1;
            continue;
        }

        let open = cursor;
        let open_line_end = line_end(source, open);
        let opener_occupies_line = source[open + 2..open_line_end].trim().is_empty();
        let mut close = open + 2;
        let mut matched = None;

        while close + 1 < bytes.len() {
            if bytes[close] == b'\\'
                && bytes[close + 1] == b']'
                && is_unescaped_backslash(bytes, close)
            {
                let close_line_end = line_end(source, close);
                let same_line = close < open_line_end;
                let standalone_close = line_prefix_is_whitespace(source, close)
                    && source[close + 2..close_line_end].trim().is_empty();
                let standalone_same_line =
                    same_line && source[close + 2..open_line_end].trim().is_empty();

                if standalone_same_line || (!same_line && opener_occupies_line && standalone_close)
                {
                    matched = Some(close);
                    break;
                }
            }
            close += 1;
        }

        let Some(close) = matched else {
            cursor = open + 2;
            continue;
        };
        let range = open..close + 2;
        let overlaps_protected = protected
            .iter()
            .any(|barrier| barrier.start < range.end && barrier.end > range.start);
        if !overlaps_protected && safe_dollar_fence_contents(&source[open + 2..close]) {
            candidates.push(LatexMathCandidate {
                range,
                mode: LatexMathMode::Display,
            });
        }
        cursor = close + 2;
    }
}

fn line_prefix_is_whitespace(source: &str, index: usize) -> bool {
    let start = source[..index]
        .rfind('\n')
        .map_or(0, |line_break| line_break + 1);
    source[start..index].trim().is_empty()
}

fn line_end(source: &str, index: usize) -> usize {
    source[index..]
        .find('\n')
        .map_or(source.len(), |line_break| index + line_break)
}

fn collect_inline_candidates(
    source: &str,
    range: Range<usize>,
    candidates: &mut Vec<LatexMathCandidate>,
) {
    let bytes = source.as_bytes();
    let mut cursor = range.start;
    while cursor + 1 < range.end {
        if bytes[cursor] != b'\\'
            || bytes[cursor + 1] != b'('
            || !is_unescaped_backslash(bytes, cursor)
        {
            cursor += 1;
            continue;
        }

        let mut close = cursor + 2;
        let mut matched = None;
        while close + 1 < range.end {
            if bytes[close] == b'\\'
                && bytes[close + 1] == b')'
                && is_unescaped_backslash(bytes, close)
            {
                matched = Some(close);
                break;
            }
            close += 1;
        }

        let Some(close) = matched else {
            break;
        };
        if safe_dollar_fence_contents(&source[cursor + 2..close]) {
            candidates.push(LatexMathCandidate {
                range: cursor..close + 2,
                mode: LatexMathMode::Inline,
            });
        }
        cursor = close + 2;
    }
}

fn safe_dollar_fence_contents(contents: &str) -> bool {
    !contents.is_empty()
        && !contents.starts_with('$')
        && !contents.ends_with('$')
        && !contents.contains("$$")
}

fn is_unescaped_backslash(bytes: &[u8], index: usize) -> bool {
    let preceding = bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    preceding % 2 == 0
}

fn normalize_latex_math_candidates(source: &str, candidates: &[LatexMathCandidate]) -> String {
    let mut normalized = source.as_bytes().to_vec();
    for candidate in candidates {
        normalized[candidate.range.start..candidate.range.start + 2].copy_from_slice(b"$$");
        normalized[candidate.range.end - 2..candidate.range.end].copy_from_slice(b"$$");
    }
    String::from_utf8(normalized).expect("ASCII delimiter replacement must preserve UTF-8")
}

fn parsed_math_ranges(root: &mdast::Node) -> Vec<(Range<usize>, bool)> {
    fn visit(node: &mdast::Node, ranges: &mut Vec<(Range<usize>, bool)>) {
        match node {
            mdast::Node::InlineMath(_) => {
                if let Some(position) = node.position() {
                    ranges.push((position.start.offset..position.end.offset, false));
                }
            }
            mdast::Node::Math(_) => {
                if let Some(position) = node.position() {
                    ranges.push((position.start.offset..position.end.offset, true));
                }
            }
            _ => {}
        }
        if let Some(children) = node.children() {
            for child in children {
                visit(child, ranges);
            }
        }
    }

    let mut ranges = Vec::new();
    visit(root, &mut ranges);
    ranges
}
