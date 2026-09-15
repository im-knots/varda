//! Per-fixture role values: the currency between the merge engine and the output path.
//!
//! A dense array plus a presence bitmask rather than a map. This is touched once per fixture
//! per role per frame at up to 44 Hz across potentially hundreds of fixtures, so the hashing
//! and allocation a map would cost is real, and 31 roles fit a `u32` mask exactly.

use super::role::Role;

/// Normalized values for one fixture, with absent roles distinguished from zero.
///
/// The distinction matters: a role nothing drives falls back to the patch default, while a role
/// driven to zero is genuinely dark. Collapsing the two would override every mode and macro
/// channel an operator pinned.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RoleValues {
    present: u32,
    values: [f32; Role::COUNT],
}

impl Default for RoleValues {
    fn default() -> Self {
        Self {
            present: 0,
            values: [0.0; Role::COUNT],
        }
    }
}

impl RoleValues {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn get(&self, role: Role) -> Option<f32> {
        let i = role.index();
        (self.present & (1 << i) != 0).then(|| self.values[i])
    }

    pub fn set(&mut self, role: Role, value: f32) {
        let i = role.index();
        self.present |= 1 << i;
        self.values[i] = value;
    }

    pub fn clear(&mut self, role: Role) {
        self.present &= !(1 << role.index());
    }

    #[must_use]
    pub fn contains(&self, role: Role) -> bool {
        self.present & (1 << role.index()) != 0
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.present == 0
    }

    /// Every role carrying a value, in [`Role::ALL`] order.
    pub fn iter(&self) -> impl Iterator<Item = (Role, f32)> + '_ {
        Role::ALL
            .iter()
            .copied()
            .filter_map(move |r| self.get(r).map(|v| (r, v)))
    }

    /// Drive every present role to zero, keeping presence intact.
    ///
    /// Used for blackout: the fixture must actively be told dark rather than falling back to
    /// its patch defaults, which could be lit.
    pub fn zero_all(&mut self) {
        for i in 0..Role::COUNT {
            if self.present & (1 << i) != 0 {
                self.values[i] = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn absent_is_distinct_from_zero() {
        let mut v = RoleValues::new();
        assert_eq!(v.get(Role::Red), None);
        v.set(Role::Red, 0.0);
        assert_eq!(v.get(Role::Red), Some(0.0), "explicit zero is not absence");
    }

    #[test]
    fn set_and_get_round_trip() {
        let mut v = RoleValues::new();
        v.set(Role::Pan, 0.25);
        v.set(Role::Tilt, 0.75);
        assert_eq!(v.get(Role::Pan), Some(0.25));
        assert_eq!(v.get(Role::Tilt), Some(0.75));
        assert_eq!(v.get(Role::Dimmer), None);
    }

    #[test]
    fn clear_removes_presence() {
        let mut v = RoleValues::new();
        v.set(Role::Red, 1.0);
        v.clear(Role::Red);
        assert_eq!(v.get(Role::Red), None);
        assert!(v.is_empty());
    }

    #[test]
    fn every_role_is_independently_addressable() {
        // Guards the bitmask width against a role being added past 32.
        let mut v = RoleValues::new();
        for (n, &r) in Role::ALL.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            v.set(r, n as f32);
        }
        for (n, &r) in Role::ALL.iter().enumerate() {
            #[allow(clippy::cast_precision_loss)]
            let expect = n as f32;
            assert_eq!(v.get(r), Some(expect), "{r} collided");
        }
    }

    #[test]
    fn iter_yields_only_present_roles_in_order() {
        let mut v = RoleValues::new();
        v.set(Role::Blue, 0.5);
        v.set(Role::Dimmer, 1.0);
        let got: Vec<Role> = v.iter().map(|(r, _)| r).collect();
        assert_eq!(got, vec![Role::Dimmer, Role::Blue], "Role::ALL order");
    }

    #[test]
    fn zero_all_keeps_presence() {
        let mut v = RoleValues::new();
        v.set(Role::Red, 1.0);
        v.zero_all();
        assert_eq!(
            v.get(Role::Red),
            Some(0.0),
            "blackout must actively drive dark, not fall back to patch defaults"
        );
    }
}
