use std::sync::Arc;

use gpui::{
    AnyElement, App, Element, ElementId, GlobalElementId, InspectorElementId,
    InteractiveElement as _, IntoElement, LayoutId, ObjectFit, ParentElement as _, Pixels,
    StatefulInteractiveElement as _, Styled as _, StyledImage as _, Window, div, img,
    prelude::FluentBuilder as _, relative,
};

use crate::{
    WindowExt as _,
    text::text_view::{LinkClickHandlerFn, handle_link_click},
    tooltip::Tooltip,
};

use super::{inline::Inline, inline_flow::InlineFlowItem, markdown_ext::MarkdownExtensions};

/// Native GPUI flex layout for paragraphs containing custom inline nodes.
///
/// Custom inline elements cannot participate in `StyledText`'s own line
/// wrapping. Letting GPUI lay out the text runs and fixed-size custom nodes as
/// flex items mirrors Zed's Markdown element builder and, importantly, lets
/// each text run report its real multi-line height.
pub(super) struct InlineFlex {
    id: ElementId,
    items: Vec<InlineFlowItem>,
    link_click_handler: Option<Arc<LinkClickHandlerFn>>,
    markdown_extensions: Arc<MarkdownExtensions>,
}

impl InlineFlex {
    pub(super) fn new(
        id: impl Into<ElementId>,
        items: Vec<InlineFlowItem>,
        link_click_handler: Option<Arc<LinkClickHandlerFn>>,
        markdown_extensions: Arc<MarkdownExtensions>,
    ) -> Self {
        Self {
            id: id.into(),
            items,
            link_click_handler,
            markdown_extensions,
        }
    }

    fn image_element(
        ix: usize,
        item: &InlineFlowItem,
        link_click_handler: Option<Arc<LinkClickHandlerFn>>,
    ) -> Option<AnyElement> {
        let InlineFlowItem::Image {
            url,
            link,
            title,
            width,
            height,
        } = item
        else {
            return None;
        };

        Some(
            img(url.clone())
                .id(ix)
                .object_fit(ObjectFit::Contain)
                .max_w(relative(1.))
                .when_some(*width, |this, width| this.w(width))
                .when_some(*height, |this, height| this.h(height))
                .when_some(link.clone(), |this, link| {
                    let title = title.clone();
                    let aux_link = link.clone();
                    let aux_link_click_handler = link_click_handler.clone();
                    this.cursor_pointer()
                        .tooltip(move |window, cx| Tooltip::new(title.clone()).build(window, cx))
                        .on_click(move |event, window, cx| {
                            window.end_text_selection(cx);
                            cx.stop_propagation();
                            handle_link_click(
                                &link_click_handler,
                                link.url.clone(),
                                event.clone(),
                                window,
                                cx,
                            );
                        })
                        .on_aux_click(move |event, window, cx| {
                            window.end_text_selection(cx);
                            cx.stop_propagation();
                            handle_link_click(
                                &aux_link_click_handler,
                                aux_link.url.clone(),
                                event.clone(),
                                window,
                                cx,
                            );
                        })
                })
                .into_any_element(),
        )
    }
}

impl IntoElement for InlineFlex {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for InlineFlex {
    type RequestLayoutState = AnyElement;
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        Some(self.id.clone())
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        // This must be read during layout rather than Paragraph::render so a
        // formula inside a heading or blockquote inherits the actual style at
        // its position.
        let text_style = window.text_style();
        let children = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(ix, item)| match item {
                InlineFlowItem::Text {
                    state,
                    text,
                    links,
                    highlights,
                } => {
                    if let Ok(mut state) = state.lock() {
                        state.set_text(text.clone());
                    }
                    Some(
                        div()
                            .min_w_0()
                            .max_w_full()
                            .flex_shrink_1()
                            .child(Inline::new(
                                ix,
                                state.clone(),
                                links.clone(),
                                highlights.clone(),
                                self.link_click_handler.clone(),
                            ))
                            .into_any_element(),
                    )
                }
                InlineFlowItem::Image { .. } => {
                    Self::image_element(ix, item, self.link_click_handler.clone())
                }
                InlineFlowItem::Custom { node, highlights } => {
                    let text_style = highlights
                        .iter()
                        .fold(text_style.clone(), |style, highlight| {
                            style.highlight(*highlight)
                        });
                    let element = self
                        .markdown_extensions
                        .render_inline(node, &text_style, window, cx)
                        .unwrap_or_else(|| node.as_text().to_string().into_any_element());

                    // Match Zed's PR implementation: custom inline content is
                    // a non-growing flex item centered with the surrounding
                    // text runs.
                    Some(
                        div()
                            .relative()
                            .flex_none()
                            .child(element)
                            .into_any_element(),
                    )
                }
            })
            .collect::<Vec<_>>();

        let mut element = div()
            .id(self.id.clone())
            .w_full()
            .min_w_0()
            .flex()
            .flex_wrap()
            .items_center()
            .children(children)
            .into_any_element();
        let layout_id = element.request_layout(window, cx);
        (layout_id, element)
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        request_layout.prepaint(window, cx);
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        _bounds: gpui::Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        _prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        request_layout.paint(window, cx);
    }
}
