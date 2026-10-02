//! OrbitEngine text DSL (ADR 0015 §5, task M8-06).
//!
//! The DSL is a **view** of the story IR, not a storage format: the RON IR
//! stays the single source of truth. [`print_story`] / [`print_scene`] render
//! scenes as Ren'Py-like text; [`parse`] reads such text back *against the
//! existing story* and returns the merged story; [`diff`] turns the merge
//! into [`orbit_project::Command`]s and [`apply`] runs them as one undoable
//! transaction.
//!
//! Stable ids survive the trip as discreet end-of-line annotations
//! (`#@01J…`). A line without an annotation is a new node. Everything the
//! text cannot express (definitions, string ids of unchanged lines, …) comes
//! from the base story.
//!
//! See [`printer`] for the grammar.

mod diff;
mod expr;
mod lexer;
mod parser;
pub mod printer;

pub use diff::{ApplyError, apply, diff};
pub use parser::{Diagnostic, Scope, parse};
pub use printer::{print_scene, print_story};
