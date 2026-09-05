use std::rc::Rc;

use gpui::{
    AnyElement, App, AppContext as _, Entity, InteractiveElement as _, IntoElement,
    ParentElement as _, SharedString, StyleRefinement, Styled, Subscription, Window, div,
};

use crate::{
    Disableable, Sizable, StyledExt,
    input::{InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    setting::{
        AnySettingField, RenderOptions,
        fields::{SettingFieldRender, get_value, set_value},
    },
};

#[derive(Clone, Debug)]
pub struct NumberFieldOptions {
    /// The minimum value for the number input, default is `f64::MIN`.
    pub min: f64,
    /// The maximum value for the number input, default is `f64::MAX`.
    pub max: f64,
    /// The step value for the number input, default is `1.0`.
    pub step: f64,
    /// The number of digits allowed and displayed after the decimal point.
    ///
    /// `None` preserves the value's natural string representation.
    pub decimal_places: Option<usize>,
}

impl Default for NumberFieldOptions {
    fn default() -> Self {
        Self {
            min: f64::MIN,
            max: f64::MAX,
            step: 1.0,
            decimal_places: None,
        }
    }
}

pub(crate) struct NumberField {
    options: NumberFieldOptions,
}

impl NumberField {
    pub(crate) fn new(options: Option<&NumberFieldOptions>) -> Self {
        Self {
            options: options.cloned().unwrap_or_default(),
        }
    }
}

struct State {
    input: Entity<InputState>,
    last_valid_value: f64,
    draft_dirty: bool,
    _subscriptions: Vec<Subscription>,
}

fn format_number(value: f64, decimal_places: Option<usize>) -> String {
    match decimal_places {
        Some(decimal_places) => format!("{value:.decimal_places$}"),
        None => value.to_string(),
    }
}

fn normalize_number(value: f64, options: &NumberFieldOptions) -> f64 {
    let value = value.clamp(options.min, options.max);
    let value = options
        .decimal_places
        .and_then(|decimal_places| format_number(value, Some(decimal_places)).parse().ok())
        .unwrap_or(value);
    value.clamp(options.min, options.max)
}

fn is_live_value(value: f64, options: &NumberFieldOptions) -> bool {
    value.is_finite() && value >= options.min && value <= options.max
}

fn has_at_most_decimal_places(value: &str, decimal_places: usize) -> bool {
    value.split_once('.').map_or(true, |(_, fraction)| {
        fraction.chars().count() <= decimal_places
    })
}

fn finish_edit(
    state: &mut State,
    input: &Entity<InputState>,
    options: &NumberFieldOptions,
    set_value: &Rc<dyn Fn(f64, &mut App)>,
    window: &mut Window,
    cx: &mut App,
) {
    let draft = input.read(cx).value();
    let value = draft
        .parse::<f64>()
        .ok()
        .filter(|value| value.is_finite())
        .map(|value| normalize_number(value, options));
    let value = value.unwrap_or(state.last_valid_value);
    let formatted = format_number(value, options.decimal_places);

    input.update(cx, |input, cx| {
        if input.value().as_ref() != formatted {
            input.set_value(SharedString::from(formatted), window, cx);
        }
    });
    if value != state.last_valid_value {
        set_value(value, cx);
    }
    state.last_valid_value = value;
    state.draft_dirty = false;
}

impl SettingFieldRender for NumberField {
    fn render(
        &self,
        field: Rc<dyn AnySettingField>,
        options: &RenderOptions,
        style: &StyleRefinement,
        window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let value = get_value::<f64>(&field, cx);
        let set_value = set_value::<f64>(&field, cx);
        let step_set_value = set_value.clone();
        let input_options = self.options.clone();
        let step_options = self.options.clone();
        let edit_options = self.options.clone();

        let state_entity = window.use_keyed_state(
            SharedString::from(format!(
                "number-state-{}-{}-{}",
                options.page_ix(),
                options.group_ix(),
                options.item_ix()
            )),
            cx,
            |window, cx| {
                let initial_text = format_number(value, input_options.decimal_places);
                let input = cx.new(|cx| {
                    let mut input = InputState::new(window, cx)
                        .default_value(initial_text)
                        .min(input_options.min)
                        .max(input_options.max);
                    if let Some(decimal_places) = input_options.decimal_places {
                        input = input.validate(move |value, _| {
                            has_at_most_decimal_places(value, decimal_places)
                        });
                    }
                    input
                });
                // Setting fields own stepping so that their configured step and
                // display precision are applied together. InputState otherwise
                // uses its built-in default step of 1.
                input.update(cx, |input, cx| input.set_step(None, window, cx));
                let _subscriptions = vec![
                    cx.subscribe_in(&input, window, {
                        move |state: &mut State, input, event: &NumberInputEvent, window, cx| {
                            match event {
                                NumberInputEvent::Step(action) => {
                                    let current = input
                                        .read(cx)
                                        .value()
                                        .parse::<f64>()
                                        .ok()
                                        .filter(|value| value.is_finite())
                                        .unwrap_or(state.last_valid_value);
                                    let stepped = match action {
                                        StepAction::Increment => current + step_options.step,
                                        StepAction::Decrement => current - step_options.step,
                                    };
                                    let stepped = normalize_number(stepped, &step_options);
                                    let formatted =
                                        format_number(stepped, step_options.decimal_places);
                                    input.update(cx, |input, cx| {
                                        if input.value().as_ref() != formatted {
                                            input.set_value(
                                                SharedString::from(formatted),
                                                window,
                                                cx,
                                            );
                                        }
                                    });
                                    if stepped != state.last_valid_value {
                                        step_set_value(stepped, cx);
                                    }
                                    state.last_valid_value = stepped;
                                    state.draft_dirty = false;
                                }
                            }
                        }
                    }),
                    cx.subscribe_in(&input, window, {
                        move |state: &mut State, input, event: &InputEvent, window, cx| match event
                        {
                            InputEvent::Change => {
                                state.draft_dirty = true;
                                let draft = input.read(cx).value();
                                if let Ok(value) = draft.parse::<f64>()
                                    && is_live_value(value, &edit_options)
                                {
                                    set_value(value, cx);
                                    state.last_valid_value = value;
                                }
                            }
                            InputEvent::PressEnter { .. } | InputEvent::Blur => {
                                finish_edit(state, input, &edit_options, &set_value, window, cx)
                            }
                            _ => {}
                        }
                    }),
                ];

                State {
                    input,
                    last_valid_value: value,
                    draft_dirty: false,
                    _subscriptions,
                }
            },
        );

        // Sync the displayed value when the underlying setting changed externally
        state_entity.update(cx, |state, cx| {
            let external = format_number(value, self.options.decimal_places);
            let current = format_number(state.last_valid_value, self.options.decimal_places);
            if !state.draft_dirty && external != current {
                state.last_valid_value = value;
                state.input.update(cx, |input, cx| {
                    input.set_value(SharedString::from(external), window, cx);
                });
            } else if !state.draft_dirty {
                // Keep the exact external value for a later restore without
                // exposing f32-to-f64 representation noise in the editor.
                state.last_valid_value = value;
            } else {
                // Preserve the draft, but restore the latest external value if
                // the draft is later abandoned or cannot be parsed.
                state.last_valid_value = value;
            }
        });

        let state = state_entity.read(cx);
        let selector = format!(
            "setting-number-input-{}-{}-{}",
            options.page_ix(),
            options.group_ix(),
            options.item_ix()
        );

        div()
            .debug_selector(move || selector.clone())
            .w_32()
            .flex_none()
            .child(
                NumberInput::new(&state.input)
                    .disabled(options.is_disabled())
                    .with_size(options.size())
                    .w_full()
                    .flex_none()
                    .refine_style(style),
            )
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decimal_options() -> NumberFieldOptions {
        NumberFieldOptions {
            min: 0.0,
            max: 10.0,
            step: 0.05,
            decimal_places: Some(2),
        }
    }

    #[test]
    fn number_formatting_hides_float_representation_noise() {
        assert_eq!(format_number(1.1_f32 as f64, Some(2)), "1.10");
        assert_eq!(format_number(12.0, Some(2)), "12.00");
        assert_eq!(format_number(12.0, None), "12");
    }

    #[test]
    fn normalization_rounds_to_display_precision_and_clamps() {
        let options = decimal_options();
        assert_eq!(normalize_number(1.09999, &options), 1.1);
        assert_eq!(normalize_number(12.0, &options), 10.0);
        assert_eq!(normalize_number(-1.0, &options), 0.0);
    }

    #[test]
    fn decimal_validation_allows_drafts_but_limits_fraction_digits() {
        assert!(has_at_most_decimal_places("", 2));
        assert!(has_at_most_decimal_places("1.", 2));
        assert!(has_at_most_decimal_places("1.10", 2));
        assert!(!has_at_most_decimal_places("1.100", 2));
    }
}
