use std::{rc::Rc, time::Duration};

use gpui::{
    Animation, AnimationExt, ElementId, Hsla, IntoElement, Pixels, Point, Styled, point,
    prelude::FluentBuilder, px,
};
use smallvec::SmallVec;

/// A cubic bezier function like CSS `cubic-bezier`.
///
/// Builder:
///
/// https://cubic-bezier.com
pub fn cubic_bezier(x1: f32, y1: f32, x2: f32, y2: f32) -> impl Fn(f32) -> f32 {
    let coefficients = |first: f32, second: f32| {
        let c = 3.0 * first;
        let b = 3.0 * (second - first) - c;
        let a = 1.0 - c - b;
        (a, b, c)
    };
    let (ax, bx, cx) = coefficients(x1, x2);
    let (ay, by, cy) = coefficients(y1, y2);

    move |progress: f32| {
        let progress = progress.clamp(0.0, 1.0);
        if progress == 0.0 || progress == 1.0 {
            return progress;
        }

        let sample_x = |t: f32| ((ax * t + bx) * t + cx) * t;
        let sample_y = |t: f32| ((ay * t + by) * t + cy) * t;
        let derivative_x = |t: f32| (3.0 * ax * t + 2.0 * bx) * t + cx;

        let mut t = progress;
        let mut converged = false;
        for _ in 0..8 {
            let error = sample_x(t) - progress;
            if error.abs() < 1e-6 {
                converged = true;
                break;
            }
            let derivative = derivative_x(t);
            if derivative.abs() < 1e-6 {
                break;
            }
            t = (t - error / derivative).clamp(0.0, 1.0);
        }
        if !converged {
            let (mut lower, mut upper) = (0.0, 1.0);
            for _ in 0..24 {
                let middle = (lower + upper) * 0.5;
                if sample_x(middle) < progress {
                    lower = middle;
                } else {
                    upper = middle;
                }
            }
            t = (lower + upper) * 0.5;
        }

        sample_y(t).clamp(0.0, 1.0)
    }
}

// ── Easing presets ──────────────────────────────────────────────────────────

/// Cubic ease-out — fast start, slow end. Good for enter animations.
pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Cubic ease-in — slow start, fast end. Good for exit animations.
pub fn ease_in_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    t * t * t
}

/// Cubic ease-in-out — slow start and end. Good for position transitions.
pub fn ease_in_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 {
        4.0 * t * t * t
    } else {
        1.0 - (-2.0 * t + 2.0).powi(3) / 2.0
    }
}

// ── Lerp trait ──────────────────────────────────────────────────────────────

/// Trait for types that support linear interpolation.
pub trait Lerp: Clone {
    fn lerp(&self, target: &Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(&self, target: &Self, t: f32) -> Self {
        self + (target - self) * t
    }
}

impl Lerp for Pixels {
    fn lerp(&self, target: &Self, t: f32) -> Self {
        let a: f32 = (*self).into();
        let b: f32 = (*target).into();
        px(a + (b - a) * t)
    }
}

impl Lerp for Point<Pixels> {
    fn lerp(&self, target: &Self, t: f32) -> Self {
        point(
            Lerp::lerp(&self.x, &target.x, t),
            Lerp::lerp(&self.y, &target.y, t),
        )
    }
}

impl Lerp for Hsla {
    /// Interpolate each channel linearly. Intended for transitions between
    /// near-grayscale UI colors (e.g. text colors), where hue interpolation is
    /// irrelevant.
    fn lerp(&self, target: &Self, t: f32) -> Self {
        Hsla {
            h: self.h.lerp(&target.h, t),
            s: self.s.lerp(&target.s, t),
            l: self.l.lerp(&target.l, t),
            a: self.a.lerp(&target.a, t),
        }
    }
}

// ── Transition combinator ───────────────────────────────────────────────────

/// A composable transition that describes animated style changes.
///
/// # Example
///
/// ```ignore
/// Transition::new(Duration::from_millis(150))
///     .ease(ease_out_cubic)
///     .slide_y(px(-4.), px(0.))
///     .fade(0.0, 1.0)
///     .apply(element, "enter-anim")
/// ```
#[derive(Clone)]
pub struct Transition {
    pub duration: Duration,
    easing: Rc<dyn Fn(f32) -> f32>,
    effects: SmallVec<[TransitionEffect; 2]>,
}

#[derive(Clone, Copy)]
enum TransitionEffect {
    SlideY(Pixels, Pixels),
    SlideX(Pixels, Pixels),
    Fade(f32, f32),
    Width(Pixels, Pixels),
    Height(Pixels, Pixels),
}

impl Transition {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            easing: Rc::new(ease_out_cubic),
            effects: SmallVec::new(),
        }
    }

    /// Set the easing function.
    pub fn ease(mut self, easing: impl Fn(f32) -> f32 + 'static) -> Self {
        self.easing = Rc::new(easing);
        self
    }

    /// Animate vertical offset from `from` to `to`.
    pub fn slide_y(mut self, from: Pixels, to: Pixels) -> Self {
        self.effects.push(TransitionEffect::SlideY(from, to));
        self
    }

    /// Animate horizontal offset from `from` to `to`.
    pub fn slide_x(mut self, from: Pixels, to: Pixels) -> Self {
        self.effects.push(TransitionEffect::SlideX(from, to));
        self
    }

    /// Animate opacity from `from` to `to`.
    pub fn fade(mut self, from: f32, to: f32) -> Self {
        self.effects.push(TransitionEffect::Fade(from, to));
        self
    }

    /// Animate width from `from` to `to`.
    pub fn width(mut self, from: Pixels, to: Pixels) -> Self {
        self.effects.push(TransitionEffect::Width(from, to));
        self
    }

    /// Animate height from `from` to `to`.
    pub fn height(mut self, from: Pixels, to: Pixels) -> Self {
        self.effects.push(TransitionEffect::Height(from, to));
        self
    }

    /// Apply this transition to a Styled element, returning an AnimationElement.
    pub fn apply<E: IntoElement + Styled + 'static>(
        self,
        element: E,
        id: impl Into<ElementId>,
    ) -> gpui::AnimationElement<E> {
        let animation = Animation::new(self.duration).with_easing({
            let easing = self.easing.clone();
            move |t| easing(t)
        });
        let effects = self.effects;
        element.with_animation(id, animation, move |el, delta| {
            let mut el = el;
            for effect in &effects {
                match effect {
                    TransitionEffect::SlideY(from, to) => {
                        el = el.top(Lerp::lerp(from, to, delta));
                    }
                    TransitionEffect::SlideX(from, to) => {
                        el = el.left(Lerp::lerp(from, to, delta));
                    }
                    TransitionEffect::Fade(from, to) => {
                        el = el.opacity(Lerp::lerp(from, to, delta));
                    }
                    TransitionEffect::Width(from, to) => {
                        el = el.w(Lerp::lerp(from, to, delta));
                    }
                    TransitionEffect::Height(from, to) => {
                        el = el.h(Lerp::lerp(from, to, delta));
                    }
                }
            }
            el
        })
    }
}

impl FluentBuilder for Transition {}

#[cfg(test)]
mod tests {
    use super::cubic_bezier;

    #[test]
    fn cubic_bezier_matches_css_timing_progress() {
        let ease = cubic_bezier(0.25, 0.1, 0.25, 1.0);
        assert!((ease(0.5) - 0.8024).abs() < 0.002);
        assert_eq!(ease(0.0), 0.0);
        assert_eq!(ease(1.0), 1.0);
    }
}
