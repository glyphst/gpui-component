use std::ops::Range;

use gpui::SharedString;
use ropey::Rope;

use crate::IconNamed;

/// A source-backed rich span rendered inside an [`InputState`](super::InputState).
///
/// The input value always retains the original text in `range`. The renderer
/// presents that source range as an atomic pill containing an optional icon,
/// `label`, and an optional editable source-backed value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InputSpan {
    pub(crate) range: Range<usize>,
    pub(crate) label: SharedString,
    pub(crate) icon_path: Option<SharedString>,
    pub(crate) editable: Option<InputSpanEditable>,
}

impl InputSpan {
    pub fn new(range: Range<usize>, label: impl Into<SharedString>) -> Self {
        Self {
            range,
            label: label.into(),
            icon_path: None,
            editable: None,
        }
    }

    /// Set an icon rendered before the span label.
    pub fn icon(mut self, icon: impl IconNamed) -> Self {
        self.icon_path = Some(icon.path());
        self
    }

    /// Make a source subrange editable as a bounded unsigned integer.
    ///
    /// Non-digit input is ignored. Empty and out-of-range values are clamped
    /// when span editing finishes.
    pub fn editable_unsigned_integer(
        mut self,
        range: Range<usize>,
        min: usize,
        max: usize,
    ) -> Self {
        self.editable = Some(InputSpanEditable {
            range,
            mode: InputSpanEditMode::UnsignedInteger {
                min,
                max: max.max(min),
            },
        });
        self
    }

    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    pub fn editable_range(&self) -> Option<&Range<usize>> {
        self.editable.as_ref().map(|editable| &editable.range)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct InputSpanEditable {
    pub(crate) range: Range<usize>,
    pub(crate) mode: InputSpanEditMode,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum InputSpanEditMode {
    UnsignedInteger { min: usize, max: usize },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DisplaySpan {
    pub(crate) source_range: Range<usize>,
    pub(crate) display_range: Range<usize>,
    pub(crate) icon_range: Option<Range<usize>>,
    pub(crate) editable_source_range: Option<Range<usize>>,
    pub(crate) editable_display_range: Option<Range<usize>>,
}

/// Bidirectional projection between the input's canonical value and the rich
/// text presented by the editor.
#[derive(Clone, Debug)]
pub(crate) struct SpanPresentation {
    text: Rope,
    source_to_display: Vec<usize>,
    display_to_source: Vec<usize>,
    spans: Vec<DisplaySpan>,
}

impl SpanPresentation {
    pub(crate) fn new(source: &Rope, spans: &[InputSpan]) -> Self {
        let source = source.to_string();
        let valid_spans = valid_spans(&source, spans);
        if valid_spans.is_empty() {
            return Self::identity(&source);
        }

        let mut text = String::with_capacity(source.len() + valid_spans.len() * 12);
        let mut source_to_display = vec![0; source.len() + 1];
        let mut display_to_source = vec![0];
        let mut display_spans = Vec::with_capacity(valid_spans.len());
        let mut source_cursor = 0;

        for span in valid_spans {
            push_source_fragment(
                &mut text,
                &mut display_to_source,
                &mut source_to_display,
                &source,
                source_cursor..span.range.start,
            );

            let display_start = text.len();
            source_to_display[span.range.clone()].fill(display_start);
            push_display_fragment(
                &mut text,
                &mut display_to_source,
                "\u{00a0}\u{00a0}",
                span.range.start,
            );

            let icon_range = span.icon_path.as_ref().map(|_| {
                let start = text.len();
                push_display_fragment(
                    &mut text,
                    &mut display_to_source,
                    "\u{00a0}\u{00a0}",
                    span.range.start,
                );
                start..text.len()
            });
            if icon_range.is_some() {
                push_display_fragment(
                    &mut text,
                    &mut display_to_source,
                    "\u{00a0}",
                    span.range.start,
                );
            }
            push_display_fragment(
                &mut text,
                &mut display_to_source,
                &span.label,
                span.range.start,
            );

            let (editable_source_range, editable_display_range) =
                span.editable.as_ref().map_or((None, None), |editable| {
                    let decoration_start = text.len();
                    push_display_fragment(
                        &mut text,
                        &mut display_to_source,
                        "\u{00a0}",
                        editable.range.start,
                    );
                    source_to_display[editable.range.start] = text.len();
                    push_source_fragment(
                        &mut text,
                        &mut display_to_source,
                        &mut source_to_display,
                        &source,
                        editable.range.clone(),
                    );
                    push_display_fragment(
                        &mut text,
                        &mut display_to_source,
                        "\u{00a0}",
                        editable.range.end,
                    );
                    (
                        Some(editable.range.clone()),
                        Some(decoration_start..text.len()),
                    )
                });

            push_display_fragment(
                &mut text,
                &mut display_to_source,
                "\u{00a0}\u{00a0}",
                span.range.end,
            );
            source_to_display[span.range.end] = text.len();
            let display_end = text.len();
            let split = editable_display_range
                .as_ref()
                .map_or(display_start + (display_end - display_start) / 2, |range| {
                    range.end
                });
            display_to_source[split..=display_end].fill(span.range.end);
            display_spans.push(DisplaySpan {
                source_range: span.range.clone(),
                display_range: display_start..display_end,
                icon_range,
                editable_source_range,
                editable_display_range,
            });

            // Visual separation without changing the canonical value.
            push_display_fragment(
                &mut text,
                &mut display_to_source,
                "\u{00a0}",
                span.range.end,
            );
            source_cursor = span.range.end;
        }

        push_source_fragment(
            &mut text,
            &mut display_to_source,
            &mut source_to_display,
            &source,
            source_cursor..source.len(),
        );
        source_to_display[source.len()] = text.len();
        if let Some(last) = display_to_source.last_mut() {
            *last = source.len();
        }

        Self {
            text: Rope::from(text),
            source_to_display,
            display_to_source,
            spans: display_spans,
        }
    }

    fn identity(source: &str) -> Self {
        Self {
            text: Rope::from(source),
            source_to_display: (0..=source.len()).collect(),
            display_to_source: (0..=source.len()).collect(),
            spans: Vec::new(),
        }
    }

    pub(crate) fn text(&self) -> &Rope {
        &self.text
    }

    pub(crate) fn source_offset(&self, display_offset: usize) -> usize {
        self.display_to_source
            .get(display_offset.min(self.display_to_source.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn display_offset(&self, source_offset: usize) -> usize {
        self.source_to_display
            .get(source_offset.min(self.source_to_display.len().saturating_sub(1)))
            .copied()
            .unwrap_or(0)
    }

    pub(crate) fn display_range(&self, source_range: &Range<usize>) -> Range<usize> {
        self.display_offset(source_range.start)..self.display_offset(source_range.end)
    }

    pub(crate) fn spans(&self) -> &[DisplaySpan] {
        &self.spans
    }
}

fn valid_spans<'a>(source: &str, spans: &'a [InputSpan]) -> Vec<&'a InputSpan> {
    let mut spans = spans
        .iter()
        .filter(|span| {
            span.range.start < span.range.end
                && span.range.end <= source.len()
                && source.is_char_boundary(span.range.start)
                && source.is_char_boundary(span.range.end)
                && !source[span.range.clone()].contains('\n')
                && span.editable.as_ref().is_none_or(|editable| {
                    span.range.start <= editable.range.start
                        && editable.range.start <= editable.range.end
                        && editable.range.end <= span.range.end
                        && source.is_char_boundary(editable.range.start)
                        && source.is_char_boundary(editable.range.end)
                })
        })
        .collect::<Vec<_>>();
    spans.sort_by_key(|span| span.range.start);
    let mut cursor = 0;
    spans.retain(|span| {
        if span.range.start < cursor {
            return false;
        }
        cursor = span.range.end;
        true
    });
    spans
}

fn push_display_fragment(
    text: &mut String,
    display_to_source: &mut Vec<usize>,
    fragment: &str,
    source_offset: usize,
) {
    text.push_str(fragment);
    display_to_source.extend(std::iter::repeat_n(source_offset, fragment.len()));
}

fn push_source_fragment(
    text: &mut String,
    display_to_source: &mut Vec<usize>,
    source_to_display: &mut [usize],
    source: &str,
    range: Range<usize>,
) {
    let display_start = text.len();
    let fragment = &source[range.clone()];
    text.push_str(fragment);
    for (relative_offset, ch) in fragment.char_indices() {
        let source_offset = range.start + relative_offset;
        let display_offset = display_start + relative_offset;
        source_to_display[source_offset] = display_offset;
        for byte_ix in 1..=ch.len_utf8() {
            display_to_source.push(if byte_ix == ch.len_utf8() {
                source_offset + ch.len_utf8()
            } else {
                source_offset
            });
        }
    }
    source_to_display[range.end] = text.len();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presentation_keeps_source_offsets_for_plain_text_and_editable_values() {
        let source = Rope::from("Ask @page(12) then @selection.");
        let page_start = "Ask ".len();
        let page_end = page_start + "@page(12)".len();
        let selection_start = "Ask @page(12) then ".len();
        let selection_end = selection_start + "@selection".len();
        let presentation = SpanPresentation::new(
            &source,
            &[
                InputSpan::new(page_start..page_end, "page").editable_unsigned_integer(
                    page_start + 6..page_end - 1,
                    1,
                    20,
                ),
                InputSpan::new(selection_start..selection_end, "selection"),
            ],
        );

        assert_eq!(
            presentation.source_offset(presentation.display_offset(2)),
            2
        );
        for source_offset in page_start + 6..=page_end - 1 {
            assert_eq!(
                presentation.source_offset(presentation.display_offset(source_offset)),
                source_offset
            );
        }
        assert_eq!(
            presentation.source_offset(presentation.display_offset(selection_end)),
            selection_end
        );
        let selection_display_range = &presentation.spans()[1].display_range;
        assert!(
            presentation.display_offset(selection_end) > selection_display_range.end,
            "the caret at a span boundary must remain outside the painted pill"
        );
    }

    #[test]
    fn invalid_and_overlapping_spans_are_ignored() {
        let source = Rope::from("hello");
        let presentation = SpanPresentation::new(
            &source,
            &[
                InputSpan::new(0..3, "one"),
                InputSpan::new(2..5, "two"),
                InputSpan::new(6..7, "invalid"),
            ],
        );
        assert_eq!(presentation.spans().len(), 1);
    }
}
