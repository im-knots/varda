//! The formula stack's ISF parameter names, and a [`Stack`] built from their
//! values. `shaders/fractal_explorer.fs` declares the same names.

use super::formula::{FormulaId, Slot};
use super::stack::{CombineOp, HybridMode, SLOTS, Stack};
use super::vec3::{Mat3, Vec3};

/// Parameter name of one slot's field, `slot1_formula` to `slot6_rot_z`.
/// `slot` is 0-based; names are 1-based.
pub fn slot_param(slot: usize, field: &str) -> String {
    format!("slot{}_{field}", slot + 1)
}

/// The per-slot fields, in declaration order.
pub const SLOT_FIELDS: [&str; 10] = [
    "formula", "mode", "count", "a", "b", "c", "d", "rot_x", "rot_y", "rot_z",
];

/// The stack-wide parameters, in declaration order.
pub const STACK_PARAMS: [&str; 11] = [
    "hybrid_mode",
    "repeat_from",
    "max_iterations",
    "bailout",
    "julia_mode",
    "julia_x",
    "julia_y",
    "julia_z",
    "combine_split",
    "combine_op",
    "combine_width",
];

/// Values used when a parameter is missing, the shader's defaults: a scale
/// 2.2 Amazing Box turned by its slot rotation. Slot 2 is off but keeps
/// Gnarl's parameters for when it is turned on.
pub fn default_param(name: &str) -> f64 {
    match name {
        "slot1_formula" => f64::from(FormulaId::Box as i32),
        "slot1_a" => 2.2,
        "slot1_b" => 0.5,
        "slot1_rot_x" => 0.25,
        "slot1_rot_y" | "slot2_a" => 0.15,
        "slot3_a" | "slot4_a" | "slot5_a" | "slot6_a" => 2.0,
        "slot1_count" | "slot1_c" | "slot1_d" | "slot2_count" | "slot2_b" | "slot2_c"
        | "slot2_d" | "slot3_count" | "slot3_b" | "slot3_d" | "slot4_count" | "slot4_b"
        | "slot4_d" | "slot5_count" | "slot5_b" | "slot5_d" | "slot6_count" | "slot6_b"
        | "slot6_d" | "repeat_from" => 1.0,
        "combine_split" | "combine_width" => 4.0,
        "max_iterations" => 16.0,
        "bailout" => 100.0,
        _ => 0.0,
    }
}

/// Build a stack from parameter values. `get` returns a parameter's live
/// value, or `None` when it is not declared; missing values take the defaults
/// above. Long and bool parameters arrive as their numeric values.
pub fn stack_from_params(get: impl Fn(&str) -> Option<f64>) -> Stack {
    let value = |name: &str| get(name).unwrap_or_else(|| default_param(name));
    let slots = std::array::from_fn::<Slot, SLOTS, _>(|i| {
        let field = |f: &str| value(&slot_param(i, f));
        Slot {
            formula: FormulaId::from_index(field("formula").round() as i32),
            mode: field("mode").round() as i32,
            count: field("count").max(0.0).round() as u32,
            params: [field("a"), field("b"), field("c"), field("d")],
            rotation: Mat3::from_euler(field("rot_x"), field("rot_y"), field("rot_z")),
        }
    });
    // 1-based slot numbers in the UI.
    let slot_index = |name: &str| (value(name).round().max(1.0) as usize - 1).min(SLOTS - 1);
    Stack {
        slots,
        hybrid: if value("hybrid_mode").round() as i32 == HybridMode::Combine as i32 {
            HybridMode::Combine
        } else {
            HybridMode::Alternate
        },
        repeat_from: slot_index("repeat_from"),
        max_iterations: value("max_iterations").max(0.0) as u32,
        bailout: value("bailout"),
        julia: (value("julia_mode") > 0.5)
            .then(|| Vec3::new(value("julia_x"), value("julia_y"), value("julia_z"))),
        combine_split: slot_index("combine_split"),
        combine_op: CombineOp::from_index(value("combine_op").round() as i32),
        combine_width: value("combine_width"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn missing_parameters_give_the_turned_box() {
        let stack = stack_from_params(|_| None);
        assert_eq!(stack.slots[0].formula, FormulaId::Box);
        assert_eq!(stack.slots[0].params, [2.2, 0.5, 1.0, 1.0]);
        assert_ne!(stack.slots[0].rotation, crate::fractal::Mat3::IDENTITY);
        assert!(!stack.slots[1].is_active());
        assert!(!stack.slots[2].is_active());
        assert_eq!(stack.hybrid, HybridMode::Alternate);
        assert_eq!(stack.julia, None);
    }

    /// The default is somewhere to be inside: Find Inside finds enclosed
    /// space in it.
    #[test]
    fn the_default_stack_has_rooms() {
        let stack = stack_from_params(|_| None);
        let inside = crate::fractal::find_inside(&stack, &[(crate::fractal::Vec3::ZERO, 4.0)])
            .expect("an open point");
        assert!(inside.enclosed);
    }

    #[test]
    fn values_map_by_name_with_one_based_slot_numbers() {
        let values: HashMap<String, f64> = [
            ("slot2_formula", 2.0),
            ("slot2_count", 3.0),
            ("slot2_a", 3.0),
            ("hybrid_mode", 1.0),
            ("combine_split", 2.0),
            ("repeat_from", 2.0),
            ("julia_mode", 1.0),
            ("julia_y", 0.25),
            ("combine_op", 4.0),
        ]
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
        let stack = stack_from_params(|name| values.get(name).copied());
        assert_eq!(stack.slots[1].formula, FormulaId::Menger);
        assert_eq!(stack.slots[1].count, 3);
        assert_eq!(stack.hybrid, HybridMode::Combine);
        assert_eq!(stack.combine_split, 1);
        assert_eq!(stack.repeat_from, 1);
        assert_eq!(stack.julia, Some(Vec3::new(0.0, 0.25, 0.0)));
        assert_eq!(stack.combine_op, CombineOp::Fillet);
    }

    #[test]
    fn slot_parameter_names_are_one_based() {
        assert_eq!(slot_param(0, "formula"), "slot1_formula");
        assert_eq!(slot_param(5, "rot_z"), "slot6_rot_z");
    }
}
