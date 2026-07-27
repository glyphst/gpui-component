use anyhow::Result;
use gpui::{Context, EntityInputHandler, Pixels, Task, Window, px};
use lsp_types::{
    CompletionContext, CompletionItem, CompletionResponse, InlineCompletionContext,
    InlineCompletionItem, InlineCompletionResponse, InlineCompletionTriggerKind,
    request::Completion,
};
use ropey::Rope;
use std::{cell::RefCell, ops::Range, rc::Rc, time::Duration};

use crate::input::{
    InputState,
    popovers::{CompletionMenu, ContextMenu},
};

/// Default debounce duration for inline completions.
const DEFAULT_INLINE_COMPLETION_DEBOUNCE: Duration = Duration::from_millis(300);

/// The input event that is asking a completion provider to establish a query range.
#[derive(Clone, Copy, Debug)]
pub enum CompletionTriggerEvent<'a> {
    /// The input replaced `replaced_range` with `inserted_text`.
    TextEdit {
        replaced_range: &'a Range<usize>,
        inserted_text: &'a str,
    },
    /// The caret moved without changing the input text.
    CursorMoved,
}

/// Width constraints for a completion menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CompletionMenuOptions {
    /// The preferred minimum width, clamped to the viewport.
    pub min_width: Pixels,
    /// The maximum width for both the menu and its documentation panel.
    pub max_width: Pixels,
}

impl Default for CompletionMenuOptions {
    fn default() -> Self {
        Self {
            min_width: px(120.),
            max_width: px(320.),
        }
    }
}

/// A trait for providing code completions based on the current input state and context.
pub trait CompletionProvider {
    /// Fetches completions based on the given byte offset.
    ///
    /// - The `offset` is in bytes of current cursor.
    ///
    /// textDocument/completion
    ///
    /// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#textDocument_completion
    fn completions(
        &self,
        text: &Rope,
        offset: usize,
        trigger: CompletionContext,
        window: &mut Window,
        cx: &mut Context<InputState>,
    ) -> Task<Result<CompletionResponse>>;

    /// Fetches an inline completion suggestion for the given position.
    ///
    /// This is called after a debounce period when the user stops typing.
    /// The provider can analyze the text and cursor position to determine
    /// what inline completion suggestion to show.
    ///
    ///
    /// # Arguments
    /// * `rope` - The current text content
    /// * `offset` - The cursor position in bytes
    ///
    /// textDocument/inlineCompletion
    ///
    /// https://microsoft.github.io/language-server-protocol/specifications/lsp/3.18/specification/#textDocument_inlineCompletion
    fn inline_completion(
        &self,
        _rope: &Rope,
        _offset: usize,
        _trigger: InlineCompletionContext,
        _window: &mut Window,
        _cx: &mut Context<InputState>,
    ) -> Task<Result<InlineCompletionResponse>> {
        Task::ready(Ok(InlineCompletionResponse::Array(vec![])))
    }

    /// Returns the debounce duration for inline completions.
    ///
    /// Default: 300ms
    #[inline]
    fn inline_completion_debounce(&self) -> Duration {
        DEFAULT_INLINE_COMPLETION_DEBOUNCE
    }

    fn resolve_completions(
        &self,
        _completion_indices: Vec<usize>,
        _completions: Rc<RefCell<Box<[Completion]>>>,
        _: &mut Context<InputState>,
    ) -> Task<Result<bool>> {
        Task::ready(Ok(false))
    }

    /// Determines whether a text edit should trigger menu completions.
    ///
    /// Providers that need full-document or caret-movement context can
    /// override [`Self::completion_trigger_range`] instead.
    fn is_completion_trigger(
        &self,
        _offset: usize,
        _new_text: &str,
        _cx: &mut Context<InputState>,
    ) -> bool {
        false
    }

    /// Returns the active completion query range for the current input event.
    ///
    /// The range uses UTF-8 byte offsets into `text` and must end at `cursor`.
    /// Returning `None` closes any open completion menu. Providers that do not
    /// complete on caret movement should return `None` for
    /// [`CompletionTriggerEvent::CursorMoved`].
    fn completion_trigger_range(
        &self,
        _text: &Rope,
        cursor: usize,
        event: CompletionTriggerEvent<'_>,
        cx: &mut Context<InputState>,
    ) -> Option<Range<usize>> {
        match event {
            CompletionTriggerEvent::TextEdit {
                replaced_range,
                inserted_text,
            } if self.is_completion_trigger(replaced_range.end, inserted_text, cx) => {
                Some(replaced_range.end..cursor)
            }
            CompletionTriggerEvent::TextEdit { .. } | CompletionTriggerEvent::CursorMoved => None,
        }
    }

    /// Returns the width constraints used by this provider's completion menu.
    fn completion_menu_options(&self) -> CompletionMenuOptions {
        CompletionMenuOptions::default()
    }
}

pub(crate) struct InlineCompletion {
    /// Completion item to display as an inline completion suggestion
    pub(crate) item: Option<InlineCompletionItem>,
    /// Task for debouncing inline completion requests
    pub(crate) task: Task<Result<InlineCompletionResponse>>,
}

impl Default for InlineCompletion {
    fn default() -> Self {
        Self {
            item: None,
            task: Task::ready(Ok(InlineCompletionResponse::Array(vec![]))),
        }
    }
}

impl InputState {
    pub(crate) fn handle_completion_trigger(
        &mut self,
        range: &Range<usize>,
        new_text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.completion_inserting {
            return;
        }

        let Some(provider) = self.lsp.completion_provider.clone() else {
            return;
        };

        // Always schedule inline completion (debounced).
        // It will check if menu is open before showing the suggestion.
        self.schedule_inline_completion(window, cx);

        let cursor = self.cursor();
        let Some(trigger_range) = provider.completion_trigger_range(
            &self.text,
            cursor,
            CompletionTriggerEvent::TextEdit {
                replaced_range: range,
                inserted_text: new_text,
            },
            cx,
        ) else {
            self.hide_context_menu(cx);
            return;
        };
        self.request_menu_completions(provider, trigger_range, window, cx);
    }

    /// Re-evaluate menu completions after a user-visible caret move.
    pub(in crate::input) fn handle_completion_cursor_move(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.completion_inserting
            || !self.selected_range.is_empty()
            || !self.focus_handle.is_focused(window)
        {
            return;
        }

        let Some(provider) = self.lsp.completion_provider.clone() else {
            return;
        };
        let cursor = self.cursor();
        let Some(trigger_range) = provider.completion_trigger_range(
            &self.text,
            cursor,
            CompletionTriggerEvent::CursorMoved,
            cx,
        ) else {
            self.hide_context_menu(cx);
            return;
        };
        let trigger_changed = self
            .context_menu_content
            .as_ref()
            .and_then(|menu| match menu {
                ContextMenu::Completion(menu) => menu.read(cx).trigger_start_offset,
                ContextMenu::CodeAction(_) => None,
            })
            .is_some_and(|start| start != trigger_range.start);
        if trigger_changed {
            self.hide_context_menu(cx);
        }
        self.request_menu_completions(provider, trigger_range, window, cx);
    }

    fn request_menu_completions(
        &mut self,
        provider: Rc<dyn CompletionProvider>,
        trigger_range: Range<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let new_offset = self.cursor();
        if trigger_range.start > new_offset
            || trigger_range.end != new_offset
            || trigger_range.end > self.text.len()
        {
            self.hide_context_menu(cx);
            return;
        }

        let menu = match self.context_menu_content.as_ref() {
            Some(ContextMenu::Completion(menu)) => Some(menu),
            _ => None,
        };

        // To create or get the existing completion menu.
        let menu = match menu {
            Some(menu) => menu.clone(),
            None => {
                let menu = CompletionMenu::new(
                    cx.entity(),
                    provider.completion_menu_options(),
                    window,
                    cx,
                );
                self.context_menu_content = Some(ContextMenu::Completion(menu.clone()));
                menu
            }
        };

        let start_offset = menu
            .read(cx)
            .trigger_start_offset
            .unwrap_or(trigger_range.start);
        if new_offset < start_offset {
            self.hide_context_menu(cx);
            return;
        }

        let query = self
            .text_for_range(
                self.range_to_utf16(&(start_offset..new_offset)),
                &mut None,
                window,
                cx,
            )
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        _ = menu.update(cx, |menu, _| {
            menu.update_query(start_offset, query.clone());
        });

        let completion_context = CompletionContext {
            trigger_kind: lsp_types::CompletionTriggerKind::TRIGGER_CHARACTER,
            trigger_character: Some(query),
        };

        let provider_responses =
            provider.completions(&self.text, new_offset, completion_context, window, cx);
        self._context_menu_task = cx.spawn_in(window, async move |editor, cx| {
            let mut completions: Vec<CompletionItem> = vec![];
            if let Some(provider_responses) = provider_responses.await.ok() {
                match provider_responses {
                    CompletionResponse::Array(items) => completions.extend(items),
                    CompletionResponse::List(list) => completions.extend(list.items),
                }
            }

            if completions.is_empty() {
                _ = menu.update(cx, |menu, cx| {
                    menu.hide(cx);
                    cx.notify();
                });

                return Ok(());
            }

            editor
                .update_in(cx, |editor, window, cx| {
                    if !editor.focus_handle.is_focused(window) {
                        return;
                    }

                    _ = menu.update(cx, |menu, cx| {
                        menu.show(new_offset, completions, window, cx);
                    });

                    cx.notify();
                })
                .ok();

            Ok(())
        });
    }

    /// Schedule an inline completion request after debouncing.
    pub(crate) fn schedule_inline_completion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // Clear any existing inline completion on text change
        self.clear_inline_completion(cx);

        let Some(provider) = self.lsp.completion_provider.clone() else {
            return;
        };

        let offset = self.cursor();
        let text = self.text.clone();
        let debounce = provider.inline_completion_debounce();
        let background_executor = cx.background_executor().clone();

        self.inline_completion.task = cx.spawn_in(window, async move |editor, cx| {
            // Debounce: wait before fetching to avoid unnecessary requests while typing
            background_executor.timer(debounce).await;

            // Now fetch the inline completion after the debounce period
            let task = editor.update_in(cx, |editor, window, cx| {
                // Check if cursor has moved during debounce
                if editor.cursor() != offset {
                    return None;
                }

                // Don't fetch if completion menu is open
                if editor.is_context_menu_open(cx) {
                    return None;
                }

                let trigger = InlineCompletionContext {
                    trigger_kind: InlineCompletionTriggerKind::Automatic,
                    selected_completion_info: None,
                };

                Some(provider.inline_completion(&text, offset, trigger, window, cx))
            })?;

            let Some(task) = task else {
                return Ok(InlineCompletionResponse::Array(vec![]));
            };

            let response = task.await?;

            editor.update_in(cx, |editor, _window, cx| {
                // Only apply if cursor still hasn't moved
                if editor.cursor() != offset {
                    return;
                }

                // Don't show if completion menu opened while we were fetching
                if editor.is_context_menu_open(cx) {
                    return;
                }

                if let Some(item) = match response.clone() {
                    InlineCompletionResponse::Array(items) => items.into_iter().next(),
                    InlineCompletionResponse::List(comp_list) => comp_list.items.into_iter().next(),
                } {
                    editor.inline_completion.item = Some(item);
                    cx.notify();
                }
            })?;

            Ok(response)
        });
    }

    /// Check if an inline completion suggestion is currently displayed.
    #[inline]
    pub(crate) fn has_inline_completion(&self) -> bool {
        self.inline_completion.item.is_some()
    }

    /// Clear the inline completion suggestion.
    pub(crate) fn clear_inline_completion(&mut self, cx: &mut Context<Self>) {
        self.inline_completion = InlineCompletion::default();
        cx.notify();
    }

    /// Accept the inline completion, inserting it at the cursor position.
    /// Returns true if a completion was accepted, false if there was none.
    pub(crate) fn accept_inline_completion(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(completion_item) = self.inline_completion.item.take() else {
            return false;
        };

        let cursor = self.cursor();
        let range_utf16 = self.range_to_utf16(&(cursor..cursor));
        let completion_text = completion_item.insert_text;
        self.replace_text_in_range_silent(Some(range_utf16), &completion_text, window, cx);
        true
    }
}
