//! OrbitEngine in-game UI (minimal, M1).
//!
//! Everything is data-driven through a serializable [`Theme`] whose defaults
//! are usable as is. Widgets keep their logic (layout, hit testing, input
//! handling, typewriter advance) free of any GPU access so it can be unit
//! tested; only the `draw` methods touch the renderer.
//!
//! * [`DialogueBox`]: translucent panel, colored speaker name, typewriter
//!   text, blinking "continue" marker;
//! * [`ChoiceMenu`]: stacked buttons with mouse hover, keyboard and digit
//!   selection;
//! * [`Banner`] and [`draw_progress_bar`]: building blocks of the loading,
//!   start, end and error screens.
//!
//! Known limit: no gamepad navigation yet (the platform layer has no gamepad
//! input), see the roadmap debts.

mod banner;
mod choice;
mod dialogue;
mod theme;

pub use banner::{Banner, draw_progress_bar};
pub use choice::{ChoiceMenu, MenuInput, button_rects, hit_test};
pub use dialogue::{Advance, DialogueBox};
pub use theme::{ChoiceTheme, ScreenTheme, TextBoxTheme, Theme, UiTexts, tint};
