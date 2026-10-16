//! Knob resolution: overrides → aliases → defaults, validated and canonicalized against each
//! knob's [`KnobKind`](crate::catalog::module::KnobKind).

use std::collections::BTreeMap;

use super::resolve::ResolvedGraph;
use super::{PlanError, Selection};
use crate::catalog::module::{Module, ModuleId, ResolvedKnobs};

/// Resolve a knob to its canonical value, **validated and coerced against its
/// [`KnobKind`]**: an override (from [`Selection::knob_overrides`]) wins — matching the
/// canonical [`key`](crate::Knob::key) first, then any [`aliases`](crate::Knob::aliases) —
/// then the knob's declared [`default`](crate::Knob::default); a [`required`](crate::Knob::required)
/// knob with neither is a [`PlanError::MissingRequiredKnob`]. A non-required knob with no
/// value resolves to `None`.
///
/// The chosen value is checked against the knob's `kind` and canonicalized so what templates
/// read via `{{ knobs.key }}` is always a valid literal (e.g. a `Bool` is the bare
/// `"true"`/`"false"`, never `"True"` or `"yes"`). A value the kind cannot accept is a
/// plan-time [`PlanError::InvalidKnobValue`] rather than a malformed config that fails only
/// when the container starts.
pub(super) fn resolve_knob(
    module: &ModuleId,
    knob: &crate::catalog::module::Knob,
    overrides: Option<&BTreeMap<String, String>>,
) -> Result<Option<String>, PlanError> {
    let raw = overrides
        .and_then(|o| {
            o.get(&knob.key)
                .or_else(|| knob.aliases.iter().find_map(|alias| o.get(alias)))
        })
        .map(String::as_str)
        .or(knob.default.as_deref());
    let Some(raw) = raw else {
        if knob.required {
            return Err(PlanError::MissingRequiredKnob {
                module: module.clone(),
                key: knob.key.clone(),
            });
        }
        return Ok(None);
    };
    coerce_knob(raw, &knob.kind)
        .map(Some)
        .ok_or_else(|| PlanError::InvalidKnobValue {
            module: module.clone(),
            key: knob.key.clone(),
            value: raw.to_string(),
            kind: knob.kind.clone(),
        })
}

/// Validate `raw` against `kind` and return its canonical injected form, or `None` if the
/// value is not a valid member of the kind (the caller turns that into a
/// [`PlanError::InvalidKnobValue`]).
///
/// Coercions are deliberately liberal on input, strict on output: a `Bool` accepts the usual
/// truthy/falsey spellings but always emits the bare `true`/`false`; an `Integer`/`Port`
/// parses and range-checks but re-emits the canonical decimal; a `String` passes through; an
/// `Enum` must match one of its options exactly.
pub(super) fn coerce_knob(raw: &str, kind: &crate::catalog::module::KnobKind) -> Option<String> {
    use crate::catalog::module::KnobKind;
    match kind {
        KnobKind::String => Some(raw.to_string()),
        KnobKind::Bool => match raw.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Some("true".to_string()),
            "false" | "0" | "no" | "off" => Some("false".to_string()),
            _ => None,
        },
        KnobKind::Enum { options } => options.iter().find(|o| *o == raw).cloned(),
        KnobKind::Integer { min, max } => {
            let n: i64 = raw.trim().parse().ok()?;
            if min.is_some_and(|lo| n < lo) || max.is_some_and(|hi| n > hi) {
                return None;
            }
            Some(n.to_string())
        }
        KnobKind::Port => {
            let n: u16 = raw.trim().parse().ok()?;
            (n != 0).then(|| n.to_string())
        }
    }
}

/// Resolve every graph module's knobs (see [`resolve_knob`]), keyed by module id.
pub(super) fn resolve_module_knobs(
    graph: &ResolvedGraph,
    selection: &Selection,
) -> Result<BTreeMap<ModuleId, ResolvedKnobs>, PlanError> {
    graph
        .nodes
        .iter()
        .map(|module| {
            Ok((
                module.id().clone(),
                resolve_knobs_of(module.as_ref(), selection)?,
            ))
        })
        .collect()
}

/// Resolve one module's knobs against the selection's overrides (see [`resolve_knob`]).
pub(super) fn resolve_knobs_of(
    module: &dyn Module,
    selection: &Selection,
) -> Result<ResolvedKnobs, PlanError> {
    let overrides = selection.knob_overrides.get(module.id());
    let mut knobs = ResolvedKnobs::new();
    for knob in module.knobs() {
        if let Some(value) = resolve_knob(module.id(), knob, overrides)? {
            knobs.set(&knob.key, value);
        }
    }
    Ok(knobs)
}
