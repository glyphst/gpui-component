use gpui::{
    AnyElement, App, Axis, Div, InteractiveElement as _, IntoElement, ParentElement, Pixels,
    SharedString, Stateful, Styled, Window, div, prelude::FluentBuilder as _, px, relative,
};
use std::{any::TypeId, ops::Deref, rc::Rc};

use crate::{
    ActiveTheme as _, AxisExt, StyledExt as _,
    label::Label,
    setting::{
        AnySettingField, ElementField, RenderOptions,
        fields::{
            BoolField, DropdownField, NumberField, ResetHandler, SettingFieldRender, StringField,
        },
    },
    text::Text,
    v_flex,
};

const CONTROL_COLUMN_MAX_WIDTH: Pixels = px(480.);
const CONTROL_COLUMN_WIDTH_RATIO: f32 = 0.6;

/// Setting item.
#[derive(Clone)]
pub enum SettingItem {
    /// A normal setting item with a title, description, and field.
    Item {
        title: SharedString,
        description: Option<Text>,
        keywords: Vec<SharedString>,
        layout: Axis,
        disabled: bool,
        field: Rc<dyn AnySettingField>,
    },
    /// A full custom element to render.
    Element {
        disabled: bool,
        keywords: Vec<SharedString>,
        /// Optional custom reset behavior. The first closure reports whether
        /// the item is "dirty" (controls reset button visibility), the second
        /// performs the reset.
        reset_handler: Option<ResetHandler>,
        render: Rc<dyn Fn(&RenderOptions, &mut Window, &mut App) -> AnyElement + 'static>,
    },
}

impl SettingItem {
    /// Create a new setting item.
    pub fn new<F>(title: impl Into<SharedString>, field: F) -> Self
    where
        F: AnySettingField + 'static,
    {
        SettingItem::Item {
            title: title.into(),
            description: None,
            layout: Axis::Horizontal,
            disabled: false,
            keywords: Vec::new(),
            field: Rc::new(field),
        }
    }

    /// Create a new custom element setting item with a render closure.
    pub fn render<R, E>(render: R) -> Self
    where
        E: IntoElement,
        R: Fn(&RenderOptions, &mut Window, &mut App) -> E + 'static,
    {
        SettingItem::Element {
            disabled: false,
            keywords: Vec::new(),
            reset_handler: None,
            render: Rc::new(move |options, window, cx| {
                render(options, window, cx).into_any_element()
            }),
        }
    }

    /// Provide custom reset behavior for a custom element item.
    ///
    /// Only applies to [`SettingItem::Element`] (created via
    /// [`SettingItem::render`]). When set, the page-level reset button will
    /// appear while `is_dirty` returns true, and clicking it invokes `reset`.
    ///
    /// - `is_dirty` reports whether the item differs from its default state.
    /// - `reset` performs the reset.
    pub fn on_reset<D, R>(mut self, is_dirty: D, reset: R) -> Self
    where
        D: Fn(&App) -> bool + 'static,
        R: Fn(&mut Window, &mut App) + 'static,
    {
        match &mut self {
            SettingItem::Element { reset_handler, .. } => {
                *reset_handler = Some((Rc::new(is_dirty), Rc::new(reset)));
            }
            // `on_reset` is meaningless for a value-bearing item: use the
            // field's own `default_value` / `SettingField::on_reset` instead.
            SettingItem::Item { .. } => {
                debug_assert!(
                    false,
                    "SettingItem::on_reset only applies to SettingItem::Element; \
                     use SettingField::default_value or SettingField::on_reset for a normal item"
                );
            }
        }
        self
    }

    /// Set additional keywords used only for search matching (not rendered).
    ///
    /// For example, an item titled "Enable Two-factor auth" can be made
    /// searchable via "MFA". This is also useful for custom elements that
    /// have no title/description but should still show up in search results.
    pub fn keywords<I, S>(mut self, keywords: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<SharedString>,
    {
        let keywords: Vec<SharedString> = keywords.into_iter().map(Into::into).collect();
        match &mut self {
            SettingItem::Item { keywords: k, .. } => *k = keywords,
            SettingItem::Element { keywords: k, .. } => *k = keywords,
        }
        self
    }

    /// Set whether the setting item is disabled, default is false.
    ///
    /// A disabled item is rendered with reduced opacity. For
    /// [`SettingItem::Item`] the underlying field is also rendered in a
    /// non-interactive state. For [`SettingItem::Element`] the `disabled` flag
    /// is forwarded via [`RenderOptions::disabled`] so the custom renderer can
    /// disable its interactive controls.
    pub fn disabled(mut self, disabled: bool) -> Self {
        match &mut self {
            SettingItem::Item { disabled: d, .. } => *d = disabled,
            SettingItem::Element { disabled: d, .. } => *d = disabled,
        }
        self
    }

    /// Set the description of the setting item.
    ///
    /// Only applies to [`SettingItem::Item`].
    pub fn description(mut self, description: impl Into<Text>) -> Self {
        match &mut self {
            SettingItem::Item { description: d, .. } => {
                *d = Some(description.into());
            }
            SettingItem::Element { .. } => {}
        }
        self
    }

    /// Set the layout of the setting item.
    ///
    /// Only applies to [`SettingItem::Item`].
    pub fn layout(mut self, layout: Axis) -> Self {
        match &mut self {
            SettingItem::Item { layout: l, .. } => {
                *l = layout;
            }
            SettingItem::Element { .. } => {}
        }
        self
    }

    pub(crate) fn is_match(&self, query: &str, cx: &App) -> bool {
        match self {
            SettingItem::Item {
                title,
                description,
                keywords,
                ..
            } => {
                let q = &query.to_lowercase();
                title.to_lowercase().contains(q)
                    || description
                        .as_ref()
                        .map_or(false, |d| d.get_text(cx).to_lowercase().contains(q))
                    || keywords.iter().any(|s| s.to_lowercase().contains(q))
            }
            // We need to show all custom elements when not searching.
            SettingItem::Element { keywords, .. } => {
                let q = &query.to_lowercase();
                query.is_empty() || keywords.iter().any(|s| s.to_lowercase().contains(q))
            }
        }
    }

    pub(crate) fn is_resettable(&self, cx: &App) -> bool {
        match self {
            SettingItem::Item { field, .. } => field.is_resettable(cx),
            SettingItem::Element { reset_handler, .. } => reset_handler
                .as_ref()
                .is_some_and(|(is_dirty, _)| is_dirty(cx)),
        }
    }

    pub(crate) fn reset(&self, window: &mut Window, cx: &mut App) {
        match self {
            SettingItem::Item { field, .. } => field.reset(window, cx),
            SettingItem::Element { reset_handler, .. } => {
                if let Some((_, reset)) = reset_handler.as_ref() {
                    reset(window, cx);
                }
            }
        }
    }

    fn render_field(
        field: Rc<dyn AnySettingField>,
        options: RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> impl IntoElement {
        let field_type = field.field_type();
        let style = field.style().clone();
        let type_id = field.deref().type_id();
        let renderer: Box<dyn SettingFieldRender> = match type_id {
            t if t == std::any::TypeId::of::<bool>() => {
                Box::new(BoolField::new(field_type.is_switch()))
            }
            t if t == TypeId::of::<f64>() && field_type.is_number_input() => {
                Box::new(NumberField::new(field_type.number_input_options()))
            }
            t if t == TypeId::of::<SharedString>() && field_type.is_input() => {
                Box::new(StringField::<SharedString>::new())
            }
            t if t == TypeId::of::<String>() && field_type.is_input() => {
                Box::new(StringField::<String>::new())
            }
            t if t == TypeId::of::<SharedString>() && field_type.is_dropdown() => {
                Box::new(DropdownField::<SharedString>::new(
                    field_type.dropdown_options(),
                    field_type.dropdown_scrollable(),
                ))
            }
            t if t == TypeId::of::<String>() && field_type.is_dropdown() => {
                Box::new(DropdownField::<String>::new(
                    field_type.dropdown_options(),
                    field_type.dropdown_scrollable(),
                ))
            }
            _ if field_type.is_element() => Box::new(ElementField::new(field_type.element())),
            _ => unimplemented!("Unsupported setting type: {}", field.deref().type_name()),
        };

        renderer.render(field, &options, &style, window, cx)
    }

    pub(super) fn render_item(
        self,
        options: &RenderOptions,
        window: &mut Window,
        cx: &mut App,
    ) -> Stateful<Div> {
        let selector_suffix = format!(
            "{}-{}-{}",
            options.page_ix(),
            options.group_ix(),
            options.item_ix()
        );
        let item_selector = format!("setting-item-{selector_suffix}");
        let label_selector = format!("setting-item-label-{selector_suffix}");
        let field_selector = format!("setting-item-field-{selector_suffix}");
        let description_selector = format!("setting-item-description-{selector_suffix}");
        div()
            .id(SharedString::from(format!("item-{}", options.item_ix())))
            .debug_selector(move || item_selector.clone())
            .w_full()
            .child(match self {
                SettingItem::Item {
                    title,
                    description,
                    layout,
                    disabled,
                    field,
                    ..
                } => {
                    let layout = if options.layout().is_vertical() {
                        Axis::Vertical
                    } else {
                        layout
                    };

                    v_flex()
                        .w_full()
                        .when(disabled, |this| this.opacity(0.5))
                        .gap_1()
                        .child(
                            div()
                                .w_full()
                                .map(|this| {
                                    if layout.is_horizontal() {
                                        this.h_flex().justify_between().items_start()
                                    } else {
                                        this.v_flex().items_end()
                                    }
                                })
                                .gap_3()
                                .child(
                                    div()
                                        .debug_selector(move || label_selector.clone())
                                        .map(|this| {
                                            if layout.is_horizontal() {
                                                this.flex_1().min_w_0()
                                            } else {
                                                this.w_full()
                                            }
                                        })
                                        .child(Label::new(title).text_sm()),
                                )
                                .child(
                                    div()
                                        .id("field")
                                        .debug_selector(move || field_selector.clone())
                                        .map(|this| {
                                            if layout.is_horizontal() {
                                                this.h_flex()
                                                    .w(relative(CONTROL_COLUMN_WIDTH_RATIO))
                                                    .max_w(CONTROL_COLUMN_MAX_WIDTH)
                                                    .flex_none()
                                                    .min_w_0()
                                                    .justify_end()
                                            } else {
                                                this.h_flex()
                                                    .w_full()
                                                    .max_w(CONTROL_COLUMN_MAX_WIDTH)
                                                    .min_w_0()
                                                    .justify_end()
                                            }
                                        })
                                        .child(Self::render_field(
                                            field,
                                            options.with_layout(layout).with_disabled(disabled),
                                            window,
                                            cx,
                                        )),
                                ),
                        )
                        .when_some(description, |this, description| {
                            this.child(
                                div()
                                    .debug_selector(move || description_selector.clone())
                                    .w_full()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(description),
                            )
                        })
                        .into_any_element()
                }
                SettingItem::Element {
                    disabled, render, ..
                } => div()
                    .w_full()
                    .when(disabled, |this| this.opacity(0.5))
                    .child((render)(&options.with_disabled(disabled), window, cx))
                    .into_any_element(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::setting::{NumberFieldOptions, SettingField};
    use gpui::{Context, Modifiers, Render, TestAppContext, VisualTestContext, point, px, size};
    use std::cell::Cell;

    fn render_options(item_ix: usize, layout: Axis) -> RenderOptions {
        RenderOptions::new()
            .with_item_ix(item_ix)
            .with_layout(layout)
    }

    fn draw(cx: &mut VisualTestContext) {
        cx.run_until_parked();
        cx.update(|window, cx| {
            _ = window.draw(cx);
        });
    }

    struct HorizontalTextRows;

    impl Render for HorizontalTextRows {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let api_key_field = SettingField::<SharedString>::render(|_, _, _| {
                div()
                    .debug_selector(|| "api-key-control".to_string())
                    .w_full()
                    .h(px(32.0))
            });
            let region_field = SettingField::<SharedString>::render(|_, _, _| {
                div()
                    .debug_selector(|| "azure-region-control".to_string())
                    .w_full()
                    .h(px(32.0))
            });
            div().size_full().child(
                v_flex()
                    .w(px(1000.0))
                    .gap_4()
                    .child(
                        SettingItem::new("API Key", api_key_field)
                            .description("Stored locally.")
                            .render_item(&render_options(0, Axis::Horizontal), window, cx),
                    )
                    .child(
                        SettingItem::new("Azure region", region_field)
                            .description(
                                "Required by region-scoped Microsoft Translator resources.",
                            )
                            .render_item(&render_options(1, Axis::Horizontal), window, cx),
                    ),
            )
        }
    }

    struct NarrowVerticalTextRow;

    impl Render for NarrowVerticalTextRow {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let field = SettingField::<SharedString>::render(|_, _, _| {
                div()
                    .debug_selector(|| "narrow-vertical-text-control".to_string())
                    .w_full()
                    .h(px(32.0))
            });
            div().size_full().child(div().w(px(320.0)).child(
                SettingItem::new("API endpoint", field).render_item(
                    &render_options(0, Axis::Vertical),
                    window,
                    cx,
                ),
            ))
        }
    }

    struct VerticalTextRow;

    impl Render for VerticalTextRow {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let field = SettingField::<SharedString>::render(|_, _, _| {
                div()
                    .debug_selector(|| "vertical-text-control".to_string())
                    .w_full()
                    .h(px(32.0))
            });
            div().size_full().child(
                div().w(px(800.0)).child(
                    SettingItem::new("API endpoint", field)
                        .description("Default endpoint")
                        .render_item(&render_options(0, Axis::Vertical), window, cx),
                ),
            )
        }
    }

    struct NumberRows;

    impl Render for NumberRows {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let number_field = || {
                SettingField::number_input(
                    NumberFieldOptions {
                        min: 0.,
                        max: 1000.,
                        step: 1.,
                        decimal_places: None,
                    },
                    |_| 320.,
                    |_, _| {},
                )
            };

            v_flex()
                .w(px(800.0))
                .gap_4()
                .child(
                    SettingItem::new("Horizontal number", number_field()).render_item(
                        &render_options(0, Axis::Horizontal),
                        window,
                        cx,
                    ),
                )
                .child(
                    SettingItem::new("Vertical number", number_field()).render_item(
                        &render_options(1, Axis::Vertical),
                        window,
                        cx,
                    ),
                )
        }
    }

    struct EditableNumberRow {
        value: Rc<Cell<f64>>,
    }

    impl Render for EditableNumberRow {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let current_value = self.value.clone();
            let updated_value = self.value.clone();
            let view = cx.entity();
            let field = SettingField::number_input(
                NumberFieldOptions {
                    min: 1.01,
                    max: 4.0,
                    step: 0.05,
                    decimal_places: Some(2),
                },
                move |_| current_value.get(),
                move |value, cx| {
                    updated_value.set(value);
                    view.update(cx, |_, cx| cx.notify());
                },
            );

            div().size_full().child(div().w(px(800.0)).child(
                SettingItem::new("Zoom step", field).render_item(
                    &render_options(0, Axis::Horizontal),
                    window,
                    cx,
                ),
            ))
        }
    }

    struct DropdownRows;

    impl Render for DropdownRows {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let long_label = "Long translation model ".repeat(80);
            let long_field = SettingField::<SharedString>::dropdown(
                vec![("long".into(), long_label.into())],
                |_| "long".into(),
                |_, _| {},
            );
            let short_field = SettingField::<SharedString>::dropdown(
                vec![("short".into(), "Short".into())],
                |_| "short".into(),
                |_, _| {},
            );

            crate::v_flex()
                .w(px(620.0))
                .gap_4()
                .child(
                    SettingItem::new("Translation model", long_field).render_item(
                        &render_options(0, Axis::Horizontal),
                        window,
                        cx,
                    ),
                )
                .child(SettingItem::new("Thinking level", short_field).render_item(
                    &render_options(1, Axis::Horizontal),
                    window,
                    cx,
                ))
        }
    }

    #[gpui::test]
    fn horizontal_text_fields_share_a_bounded_control_column(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| HorizontalTextRows);
        cx.simulate_resize(size(px(1000.0), px(600.0)));
        let cx: &mut VisualTestContext = cx;
        draw(cx);

        let api_key_row = cx.debug_bounds("setting-item-0-0-0").unwrap();
        let api_key_label = cx.debug_bounds("setting-item-label-0-0-0").unwrap();
        let api_key_field = cx.debug_bounds("setting-item-field-0-0-0").unwrap();
        let api_key_description = cx.debug_bounds("setting-item-description-0-0-0").unwrap();
        let api_key_control = cx.debug_bounds("api-key-control").unwrap();
        let region_row = cx.debug_bounds("setting-item-0-0-1").unwrap();
        let region_label = cx.debug_bounds("setting-item-label-0-0-1").unwrap();
        let region_field = cx.debug_bounds("setting-item-field-0-0-1").unwrap();
        let region_description = cx.debug_bounds("setting-item-description-0-0-1").unwrap();
        let region_control = cx.debug_bounds("azure-region-control").unwrap();

        assert_eq!(api_key_field.right(), api_key_row.right());
        assert_eq!(region_field.right(), region_row.right());
        assert_eq!(api_key_field.size.width, CONTROL_COLUMN_MAX_WIDTH);
        assert_eq!(region_field.size.width, CONTROL_COLUMN_MAX_WIDTH);
        assert_eq!(api_key_field.left(), region_field.left());
        assert_eq!(api_key_field.right(), region_field.right());
        assert_eq!(api_key_control, api_key_field);
        assert_eq!(region_control, region_field);
        assert!(api_key_field.left() > api_key_label.right());
        assert!(region_field.left() > region_label.right());
        for (row, label, field, description) in [
            (
                api_key_row,
                api_key_label,
                api_key_field,
                api_key_description,
            ),
            (region_row, region_label, region_field, region_description),
        ] {
            assert_eq!(description.left(), row.left());
            assert_eq!(description.right(), row.right());
            assert!(description.top() > label.bottom());
            assert!(description.top() > field.bottom());
        }
    }

    #[gpui::test]
    fn vertical_field_uses_the_same_bounded_control_width_below_its_label(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| VerticalTextRow);
        cx.simulate_resize(size(px(1000.0), px(600.0)));
        let cx: &mut VisualTestContext = cx;
        draw(cx);

        let row = cx.debug_bounds("setting-item-0-0-0").unwrap();
        let label = cx.debug_bounds("setting-item-label-0-0-0").unwrap();
        let field = cx.debug_bounds("setting-item-field-0-0-0").unwrap();
        let description = cx.debug_bounds("setting-item-description-0-0-0").unwrap();
        let control = cx.debug_bounds("vertical-text-control").unwrap();

        assert_eq!(field.right(), row.right());
        assert_eq!(field.size.width, CONTROL_COLUMN_MAX_WIDTH);
        assert_eq!(control, field);
        assert!(field.top() > label.bottom());
        assert_eq!(description.left(), row.left());
        assert_eq!(description.right(), row.right());
        assert!(description.top() > field.bottom());
    }

    #[gpui::test]
    fn narrow_vertical_text_field_fills_the_available_row_width(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| NarrowVerticalTextRow);
        cx.simulate_resize(size(px(600.0), px(600.0)));
        let cx: &mut VisualTestContext = cx;
        draw(cx);

        let row = cx.debug_bounds("setting-item-0-0-0").unwrap();
        let field = cx.debug_bounds("setting-item-field-0-0-0").unwrap();
        let control = cx.debug_bounds("narrow-vertical-text-control").unwrap();

        assert_eq!(field.left(), row.left());
        assert_eq!(field.right(), row.right());
        assert_eq!(control, field);
    }

    #[gpui::test]
    fn number_inputs_remain_compact_in_horizontal_and_vertical_layouts(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| NumberRows);
        cx.simulate_resize(size(px(1000.0), px(600.0)));
        let cx: &mut VisualTestContext = cx;
        draw(cx);

        for (field_selector, number_selector) in [
            ("setting-item-field-0-0-0", "setting-number-input-0-0-0"),
            ("setting-item-field-0-0-1", "setting-number-input-0-0-1"),
        ] {
            let field = cx.debug_bounds(field_selector).unwrap();
            let number = cx.debug_bounds(number_selector).unwrap();
            assert_eq!(field.size.width, CONTROL_COLUMN_MAX_WIDTH);
            assert_eq!(number.size.width, px(128.0));
            assert_eq!(number.right(), field.right());
        }
    }

    #[gpui::test]
    fn number_input_preserves_an_incomplete_draft_until_it_becomes_valid(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let value = Rc::new(Cell::new(1.09999));
        let (_, cx) = cx.add_window_view({
            let value = value.clone();
            move |_, _| EditableNumberRow { value }
        });
        cx.simulate_resize(size(px(1000.0), px(600.0)));
        let cx: &mut VisualTestContext = cx;
        draw(cx);

        let number = cx.debug_bounds("setting-number-input-0-0-0").unwrap();
        cx.simulate_click(number.center(), Modifiers::default());
        cx.simulate_keystrokes("ctrl-a backspace");
        cx.simulate_input("1");
        draw(cx);
        assert!((value.get() - 1.09999).abs() < f64::EPSILON);

        cx.simulate_input(".1");
        draw(cx);

        assert!((value.get() - 1.1).abs() < f64::EPSILON);
    }

    #[gpui::test]
    fn number_input_buttons_use_the_configured_step(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let value = Rc::new(Cell::new(1.1));
        let (_, cx) = cx.add_window_view({
            let value = value.clone();
            move |_, _| EditableNumberRow { value }
        });
        cx.simulate_resize(size(px(1000.0), px(600.0)));
        let cx: &mut VisualTestContext = cx;
        draw(cx);

        let number = cx.debug_bounds("setting-number-input-0-0-0").unwrap();
        cx.simulate_click(
            point(number.right() - px(4.0), number.center().y),
            Modifiers::default(),
        );
        draw(cx);

        assert!((value.get() - 1.15).abs() < f64::EPSILON);

        let number = cx.debug_bounds("setting-number-input-0-0-0").unwrap();
        cx.simulate_click(
            point(number.left() + px(4.0), number.center().y),
            Modifiers::default(),
        );
        draw(cx);

        assert!((value.get() - 1.1).abs() < f64::EPSILON);
    }

    #[gpui::test]
    fn dropdown_uses_available_width_only_when_its_label_needs_it(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let (_, cx) = cx.add_window_view(|_, _| DropdownRows);
        cx.simulate_resize(size(px(800.0), px(600.0)));
        let cx: &mut VisualTestContext = cx;
        draw(cx);

        let long_field = cx.debug_bounds("setting-item-field-0-0-0").unwrap();
        let long_dropdown = cx.debug_bounds("setting-dropdown-0-0-0").unwrap();
        assert_eq!(long_dropdown.right(), long_field.right());
        assert_eq!(long_dropdown.size.width, long_field.size.width);
        assert!(long_dropdown.left() >= long_field.left());

        let short_field = cx.debug_bounds("setting-item-field-0-0-1").unwrap();
        let short_dropdown = cx.debug_bounds("setting-dropdown-0-0-1").unwrap();
        assert_eq!(short_dropdown.right(), short_field.right());
        assert!(short_dropdown.size.width < short_field.size.width);
    }
}
