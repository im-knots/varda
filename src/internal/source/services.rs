//! Device managers shared between source providers and the rest of the engine.

use std::any::{Any, TypeId};
use std::collections::HashMap;

/// Device managers used by more than one part of the engine, keyed by type.
#[derive(Default)]
pub struct Services {
    map: HashMap<TypeId, Box<dyn Any>>,
}

impl Services {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `service`, replacing any of the same type.
    pub fn insert<T: Any>(&mut self, service: T) {
        self.map.insert(TypeId::of::<T>(), Box::new(service));
    }

    /// Builder form of [`Self::insert`].
    #[must_use]
    pub fn with<T: Any>(mut self, service: T) -> Self {
        self.insert(service);
        self
    }

    pub fn get<T: Any>(&self) -> Option<&T> {
        self.map.get(&TypeId::of::<T>())?.downcast_ref()
    }

    pub fn get_mut<T: Any>(&mut self) -> Option<&mut T> {
        self.map.get_mut(&TypeId::of::<T>())?.downcast_mut()
    }

    /// Two different managers mutably at once.
    ///
    /// # Panics
    ///
    /// Panics if `A` and `B` are the same type.
    pub fn get2_mut<A: Any, B: Any>(&mut self) -> (Option<&mut A>, Option<&mut B>) {
        let [a, b] = self
            .map
            .get_disjoint_mut([&TypeId::of::<A>(), &TypeId::of::<B>()]);
        (
            a.and_then(|a| a.downcast_mut()),
            b.and_then(|b| b.downcast_mut()),
        )
    }

    /// Three different managers mutably at once.
    ///
    /// # Panics
    ///
    /// Panics if any two of `A`, `B` and `C` are the same type.
    pub fn get3_mut<A: Any, B: Any, C: Any>(
        &mut self,
    ) -> (Option<&mut A>, Option<&mut B>, Option<&mut C>) {
        let [a, b, c] =
            self.map
                .get_disjoint_mut([&TypeId::of::<A>(), &TypeId::of::<B>(), &TypeId::of::<C>()]);
        (
            a.and_then(|a| a.downcast_mut()),
            b.and_then(|b| b.downcast_mut()),
            c.and_then(|c| c.downcast_mut()),
        )
    }

    pub fn contains<T: Any>(&self) -> bool {
        self.map.contains_key(&TypeId::of::<T>())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn services_are_found_by_type() {
        let mut services = Services::new().with(3_u32).with(String::from("a"));
        assert_eq!(services.get::<u32>(), Some(&3));
        *services.get_mut::<String>().unwrap() += "b";
        assert_eq!(services.get::<String>().map(String::as_str), Some("ab"));
        assert!(services.get::<i64>().is_none());
        let (n, text) = services.get2_mut::<u32, String>();
        *n.unwrap() += 1;
        text.unwrap().push('c');
        assert_eq!(services.get::<u32>(), Some(&4));
    }
}
