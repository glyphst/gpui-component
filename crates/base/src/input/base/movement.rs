use crate::input::InputModeKind;
use gpui::{Context, Point, Window};

use crate::input::{
    InputBaseState, MoveDown, MoveEnd, MoveHome, MoveLeft, MovePageDown, MovePageUp, MoveRight,
    MoveToEnd, MoveToNextWord, MoveToPreviousWord, MoveToStart, MoveUp, RopeExt as _,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum MoveDirection {
    Up,
    Down,
}

impl<M: InputModeKind> InputBaseState<M> {
    /// Called after moving the cursor. Updates preferred_column if we know where the cursor now is.
    pub(super) fn update_preferred_column(&mut self) {
        let Some(last_layout) = &self.last_layout else {
            self.preferred_column = None;
            return;
        };

        let point = self.text.offset_to_point(self.cursor());
        let Some(line) = last_layout.line(point.row) else {
            self.preferred_column = None;
            return;
        };

        let display_line_start = self.presentation_text().line_start_offset(point.row);
        let display_offset = self.display_offset_for_source(self.cursor());
        let display_column = display_offset.saturating_sub(display_line_start);
        let Some(pos) =
            line.position_for_index(display_column, last_layout, self.cursor_line_end_affinity)
        else {
            self.preferred_column = None;
            return;
        };

        self.preferred_column = Some((pos.x, point.column));
    }

    /// Move the cursor to the given offset.
    ///
    /// The offset is the UTF-8 offset.
    ///
    /// Ensure the offset use self.next_boundary or self.previous_boundary to get the correct offset.
    pub(crate) fn move_to(
        &mut self,
        offset: usize,
        direction: Option<MoveDirection>,
        cx: &mut Context<Self>,
    ) {
        self.move_to_with_affinity(offset, direction, false, cx);
    }

    /// Like [`Self::move_to`], but also carries the caret's line-end affinity.
    ///
    /// A soft wrap boundary is one offset shared by the end of one visual line and the start of
    /// the next, so the offset alone cannot say where to draw the caret. Callers that resolved
    /// the offset from a visual position -- a click, a drag, a vertical move -- already know
    /// which of the two rows the user meant, and pass it here. Taking it in the same call as the
    /// move is what keeps the two from drifting apart.
    pub(crate) fn move_to_with_affinity(
        &mut self,
        offset: usize,
        direction: Option<MoveDirection>,
        line_end_affinity: bool,
        cx: &mut Context<Self>,
    ) {
        self.undo_manager.break_transaction_coalescing();
        let offset = offset.clamp(0, self.text.len());
        self.cursor_line_end_affinity = line_end_affinity;
        self.selected_range = (offset..offset).into();
        self.scroll_to(offset, direction, cx);
        self.pause_blink_cursor(cx);
        self.update_preferred_column();
        M::hide_context_menu(self, cx);
        M::clear_inline_completion(self, cx);
        cx.notify()
    }

    /// Move the cursor vertically by one line (up or down) while preserving the column if possible.
    ///
    /// move_lines: Number of lines to move vertically (positive for down, negative for up).
    pub(super) fn move_vertical(
        &mut self,
        move_lines: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_single_line() {
            return;
        }
        let Some(last_layout) = &self.last_layout else {
            return;
        };

        let offset = self.cursor();
        let was_preferred_column = self.preferred_column;

        let display_offset = self.display_offset_for_source(offset);
        // Start from the row the caret is drawn on, not the row the raw offset falls in: on a
        // soft wrap boundary those are two different rows.
        let mut display_point = self.display_map.offset_to_wrap_display_point_with_affinity(
            display_offset,
            self.cursor_line_end_affinity,
        );

        // Convert wrap row → display row (skips folded rows), move, then convert back
        let current_display_row = self
            .display_map
            .wrap_row_to_display_row(display_point.row)
            .unwrap_or_else(|| {
                self.display_map
                    .nearest_visible_display_row(display_point.row)
            });
        let max_display_row = self.display_map.display_row_count().saturating_sub(1);
        let target_display_row = current_display_row
            .saturating_add_signed(move_lines)
            .min(max_display_row);
        let target_wrap_row = self
            .display_map
            .display_row_to_wrap_row(target_display_row)
            .unwrap_or(display_point.row);

        display_point.row = target_wrap_row;
        display_point.column = 0;
        let mut new_display_offset = self.display_map.wrap_display_point_to_offset(display_point);
        let mut new_offset = self.source_offset_for_display(new_display_offset);

        let mut new_affinity = false;
        if let Some((preferred_x, column)) = was_preferred_column {
            // Get display point again to update local_row.
            let mut next_display_point = self
                .display_map
                .offset_to_wrap_display_point(new_display_offset);
            next_display_point.column = 0;
            let next_point = self
                .display_map
                .wrap_display_point_to_point(next_display_point);
            let display_line_start = self.presentation_text().line_start_offset(next_point.row);

            // If in visible range, prefer to use position to get column.
            if let Some(line) = last_layout.line(next_point.row) {
                if let Some((x, line_end_affinity)) = line.closest_index_for_position(
                    Point {
                        x: preferred_x,
                        y: next_display_point.local_row * last_layout.line_height,
                    },
                    last_layout,
                ) {
                    new_display_offset = display_line_start + x;
                    new_offset = self.source_offset_for_display(new_display_offset);
                    // Landing on a wrap boundary means the preferred column pointed past the
                    // last glyph of the target row, so the caret stays on that row.
                    new_affinity = line_end_affinity;
                }
            } else {
                // Not in visible range, use column directly.
                let max_line_len = self.presentation_text().slice_line(next_point.row).len();
                new_display_offset = display_line_start + column.min(max_line_len);
                new_offset = self.source_offset_for_display(new_display_offset);
            }
        }

        self.pause_blink_cursor(cx);
        let direction = if move_lines < 0 {
            MoveDirection::Up
        } else {
            MoveDirection::Down
        };
        self.move_to_with_affinity(new_offset, Some(direction), new_affinity, cx);
        // Set back the preferred_column
        self.preferred_column = was_preferred_column;
        M::on_cursor_moved(self, window, cx);
        cx.notify();
    }

    pub(super) fn left(&mut self, _: &MoveLeft, window: &mut Window, cx: &mut Context<Self>) {
        self.pause_blink_cursor(cx);
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor()), None, cx);
        } else {
            self.move_to(self.selected_range.start, None, cx)
        }
        M::on_cursor_moved(self, window, cx);
    }

    pub(super) fn right(&mut self, _: &MoveRight, window: &mut Window, cx: &mut Context<Self>) {
        self.pause_blink_cursor(cx);
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), None, cx);
        } else {
            self.move_to(self.selected_range.end, None, cx)
        }
        M::on_cursor_moved(self, window, cx);
    }

    pub(super) fn up(&mut self, action: &MoveUp, window: &mut Window, cx: &mut Context<Self>) {
        if M::handle_context_menu_action(self, Box::new(action.clone()), window, cx) {
            return;
        }

        if self.is_single_line() {
            return;
        }

        if !self.selected_range.is_empty() {
            self.move_to(
                self.previous_boundary(self.selected_range.start.saturating_sub(1)),
                Some(MoveDirection::Up),
                cx,
            );
        }
        self.pause_blink_cursor(cx);
        self.move_vertical(-1, window, cx);
    }

    pub(super) fn down(&mut self, action: &MoveDown, window: &mut Window, cx: &mut Context<Self>) {
        if M::handle_context_menu_action(self, Box::new(action.clone()), window, cx) {
            return;
        }

        if self.is_single_line() {
            return;
        }

        if !self.selected_range.is_empty() {
            self.move_to(
                self.next_boundary(self.selected_range.end.saturating_sub(1)),
                Some(MoveDirection::Down),
                cx,
            );
        }

        self.pause_blink_cursor(cx);
        self.move_vertical(1, window, cx);
    }

    pub(super) fn page_up(&mut self, _: &MovePageUp, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_single_line() {
            return;
        }

        let Some(last_layout) = &self.last_layout else {
            return;
        };

        let display_lines = (self.input_bounds.size.height / last_layout.line_height) as isize;
        self.move_vertical(-display_lines, window, cx);
    }

    pub(super) fn page_down(
        &mut self,
        _: &MovePageDown,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.is_single_line() {
            return;
        }

        let Some(last_layout) = &self.last_layout else {
            return;
        };

        let display_lines = (self.input_bounds.size.height / last_layout.line_height) as isize;
        self.move_vertical(display_lines, window, cx);
    }

    pub(super) fn home(&mut self, _: &MoveHome, window: &mut Window, cx: &mut Context<Self>) {
        self.pause_blink_cursor(cx);
        let offset = self.start_of_line();
        self.move_to(offset, Some(MoveDirection::Up), cx);
        M::on_cursor_moved(self, window, cx);
    }

    pub(super) fn end(&mut self, _: &MoveEnd, window: &mut Window, cx: &mut Context<Self>) {
        self.pause_blink_cursor(cx);
        let offset = self.end_of_line();
        self.move_to_with_affinity(offset, Some(MoveDirection::Down), true, cx);
        M::on_cursor_moved(self, window, cx);
    }

    pub(super) fn move_to_start(
        &mut self,
        _: &MoveToStart,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(0, None, cx);
        M::on_cursor_moved(self, window, cx);
    }

    pub(super) fn move_to_end(
        &mut self,
        _: &MoveToEnd,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_to(self.text.len(), None, cx);
        M::on_cursor_moved(self, window, cx);
    }

    pub(super) fn move_to_previous_word(
        &mut self,
        _: &MoveToPreviousWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.previous_start_of_word();
        self.move_to(offset, None, cx);
        M::on_cursor_moved(self, window, cx);
    }

    pub(super) fn move_to_next_word(
        &mut self,
        _: &MoveToNextWord,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let offset = self.next_end_of_word();
        self.move_to(offset, None, cx);
        M::on_cursor_moved(self, window, cx);
    }
}
